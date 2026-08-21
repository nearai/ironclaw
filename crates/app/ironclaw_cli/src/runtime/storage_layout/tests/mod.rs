#![allow(unused_imports)] // Scenario submodules share this private fixture prelude.

use std::fs;
#[cfg(any(unix, windows))]
use std::process::Command;
#[cfg(any(unix, windows))]
use std::thread;
#[cfg(any(unix, windows))]
use std::time::{Duration, Instant};

use ironclaw_config::{
    DeploymentSecurityEnvelope, DurableStateKind, LayoutManifest, LayoutRequirement,
    LegacyStorageSource, RebornHome, TenancyModel, WorkspaceAccessFloor,
};
use ironclaw_host_api::ids::{TenantId, UserId};

use super::*;
use super::{admission::*, filesystem::*, locks::*, model::*, mover::*};

pub(super) fn embedded_single_user_requirement() -> LayoutRequirement {
    LayoutRequirement {
        durable_state: DurableStateKind::EmbeddedLibSql,
        security: DeploymentSecurityEnvelope {
            tenancy: TenancyModel::SingleUser,
            workspace_access_floor: WorkspaceAccessFloor::SingleTrustedOperator,
        },
    }
}

#[test]
fn storage_migration_policy_defaults_to_automatic_and_rejects_unknown_values() {
    assert_eq!(
        StorageMigrationPolicy::from_environment_value(None).expect("default policy"),
        StorageMigrationPolicy::Automatic
    );
    assert_eq!(
        StorageMigrationPolicy::from_environment_value(Some(StorageMigrationPolicy::AUTOMATIC))
            .expect("explicit automatic"),
        StorageMigrationPolicy::Automatic
    );
    assert_eq!(
        StorageMigrationPolicy::from_environment_value(Some(StorageMigrationPolicy::MANUAL))
            .expect("explicit manual"),
        StorageMigrationPolicy::Manual
    );
    let error = StorageMigrationPolicy::from_environment_value(Some("true"))
        .expect_err("generic truthy values are not a policy");
    assert!(error.to_string().contains(StorageMigrationPolicy::ENV));
}

#[cfg(any(unix, windows))]
#[test]
fn advisory_lock_holder_subprocess() {
    let Ok(lock_root) = std::env::var("IRONCLAW_TEST_MIGRATION_LOCK_ROOT") else {
        return;
    };
    let ready = std::env::var("IRONCLAW_TEST_MIGRATION_LOCK_READY").expect("lock holder ready");
    let release =
        std::env::var("IRONCLAW_TEST_MIGRATION_LOCK_RELEASE").expect("lock holder release");
    let _lock = acquire_named_lock(
        std::path::Path::new(&lock_root),
        MIGRATION_LOCK_FILE,
        "storage layout migration",
    )
    .expect("subprocess holds migration lock");
    fs::write(ready, b"ready").expect("signal held lock");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !std::path::Path::new(&release).is_file() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        std::path::Path::new(&release).is_file(),
        "parent did not release lock holder within the bounded test interval"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn advisory_lock_recovers_after_a_terminated_process() {
    let temp = tempfile::tempdir().expect("tempdir");
    let lock_root = temp.path().join("home");
    fs::create_dir(&lock_root).expect("lock root");
    let ready = temp.path().join("lock-ready");
    let test_binary = std::env::current_exe().expect("test binary");
    let mut child = Command::new(test_binary)
        .args([
            "--exact",
            "runtime::storage_layout::tests::advisory_lock_holder_subprocess",
            "--nocapture",
        ])
        .env("IRONCLAW_TEST_MIGRATION_LOCK_ROOT", &lock_root)
        .env("IRONCLAW_TEST_MIGRATION_LOCK_READY", &ready)
        .env(
            "IRONCLAW_TEST_MIGRATION_LOCK_RELEASE",
            temp.path().join("lock-release"),
        )
        .spawn()
        .expect("spawn lock holder");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.is_file(), "lock holder reached its critical section");
    let contention =
        match acquire_named_lock(&lock_root, MIGRATION_LOCK_FILE, "storage layout migration") {
            Ok(_) => panic!("live lock holder prevents concurrent migration"),
            Err(error) => error,
        };
    assert!(
        format!("{contention:#}").contains("storage layout migration"),
        "{contention:#}"
    );
    child.kill().expect("terminate lock holder");
    let _status = child.wait().expect("reap terminated lock holder");

    let _lock = acquire_named_lock(&lock_root, MIGRATION_LOCK_FILE, "storage layout migration")
        .expect("OS advisory lock is released after a holder process is terminated");
}

pub(super) fn reborn_home(path: &std::path::Path) -> RebornHome {
    RebornHome::resolve_from_env_parts(Some(path.as_os_str().to_os_string()), None, None)
        .expect("test Reborn home")
}

pub(super) fn seed_legacy_embedded_store(root: &std::path::Path) {
    fs::create_dir_all(root).expect("legacy root");
    let key = ironclaw_secrets::keychain::generate_master_key_hex();
    fs::write(
        root.join(ironclaw_composition::STANDALONE_SECRETS_MASTER_KEY_PATH),
        key,
    )
    .expect("legacy key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(
            root.join(ironclaw_composition::STANDALONE_SECRETS_MASTER_KEY_PATH),
            fs::Permissions::from_mode(0o600),
        )
        .expect("owner-only legacy key");
    }
    crate::runtime::block_on_cli({
        let root = root.to_path_buf();
        async move {
            ironclaw_composition::open_standalone_secret_store(&root)
                .await
                .map(|_| ())
        }
    })
    .expect("seed legacy libSQL store");
}

/// Backdate every mtime-bearing file of a seeded legacy root so recency
/// ranking can order candidates deterministically.
pub(super) fn age_legacy_store(root: &std::path::Path, seconds: u64) {
    let stamp = std::time::SystemTime::now() - std::time::Duration::from_secs(seconds);
    for entry in fs::read_dir(root).expect("read legacy root") {
        let entry = entry.expect("legacy entry");
        if entry.file_type().expect("entry type").is_file() {
            let file = fs::OpenOptions::new()
                .append(true)
                .open(entry.path())
                .expect("open for timestamp");
            file.set_modified(stamp).expect("backdate mtime");
        }
    }
}

mod admission;
mod filesystem_security;
mod mover_migration;
