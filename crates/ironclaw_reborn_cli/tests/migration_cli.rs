#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HANDSHAKE_SCHEMA: &str = "ironclaw.reborn.migration-companion/v1";

fn source_reborn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ironclaw-reborn")
}

struct InstalledPair {
    _temp: tempfile::TempDir,
    reborn: PathBuf,
    capture: PathBuf,
}

impl InstalledPair {
    fn with_companion() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir(&bin_dir).expect("bin dir");
        set_mode(&bin_dir, 0o755);
        let reborn = bin_dir.join("ironclaw-reborn");
        std::fs::copy(source_reborn_bin(), &reborn).expect("copy Reborn CLI");
        set_mode(&reborn, 0o755);

        let companion = bin_dir.join("ironclaw-reborn-migration");
        std::fs::write(
            &companion,
            "#!/bin/sh\n\
             if [ \"${1:-}\" = \"__handshake\" ]; then\n\
               printf '%s\\n' \"$MIGRATION_TEST_HANDSHAKE\"\n\
               exit 0\n\
             fi\n\
             printf '%s\\n' \"$@\" > \"$MIGRATION_TEST_CAPTURE\"\n\
             exit \"$MIGRATION_TEST_EXIT_CODE\"\n",
        )
        .expect("write companion");
        set_mode(&companion, 0o755);

        let capture = temp.path().join("args.txt");
        Self {
            _temp: temp,
            reborn,
            capture,
        }
    }

    fn without_companion() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir(&bin_dir).expect("bin dir");
        set_mode(&bin_dir, 0o755);
        let reborn = bin_dir.join("ironclaw-reborn");
        std::fs::copy(source_reborn_bin(), &reborn).expect("copy Reborn CLI");
        set_mode(&reborn, 0o755);
        let capture = temp.path().join("args.txt");
        Self {
            _temp: temp,
            reborn,
            capture,
        }
    }

    fn run(&self, version: &str, exit_code: i32, args: &[&str]) -> Output {
        let mut command = Command::new(&self.reborn);
        command
            .args(args)
            .current_dir(self._temp.path())
            .env_clear()
            .env("HOME", self._temp.path().join("home"))
            .env("MIGRATION_TEST_CAPTURE", &self.capture)
            .env("MIGRATION_TEST_EXIT_CODE", exit_code.to_string())
            .env("MIGRATION_TEST_HANDSHAKE", handshake_document(version));
        command.output().expect("run copied Reborn CLI")
    }
}

fn set_mode(path: &Path, mode: u32) {
    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions).expect("set permissions");
}

fn handshake_document(version: &str) -> String {
    serde_json::json!({
        "schema_version": HANDSHAKE_SCHEMA,
        "protocol_version": 1,
        "release_version": version,
    })
    .to_string()
}

#[test]
fn migrate_v1_requires_an_explicit_operation() {
    let pair = InstalledPair::without_companion();
    let output = pair.run(env!("CARGO_PKG_VERSION"), 0, &["migrate", "v1"]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Usage:"), "stderr: {stderr}");
    assert!(stderr.contains("<COMMAND>"), "stderr: {stderr}");
}

#[test]
fn migrate_rejects_a_missing_sibling_without_searching_path() {
    let pair = InstalledPair::without_companion();
    let other_bin = pair._temp.path().join("other-bin");
    std::fs::create_dir(&other_bin).expect("other bin");
    let path_companion = other_bin.join("ironclaw-reborn-migration");
    std::fs::write(&path_companion, "#!/bin/sh\nexit 0\n").expect("PATH companion");
    set_mode(&path_companion, 0o755);

    let output = Command::new(&pair.reborn)
        .args(["migrate", "v1", "status", "--manifest", "manifest.json"])
        .current_dir(pair._temp.path())
        .env_clear()
        .env("HOME", pair._temp.path().join("home"))
        .env("PATH", &other_bin)
        .output()
        .expect("run Reborn CLI");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("companion is missing"), "stderr: {stderr}");
    assert!(
        stderr.contains(pair.reborn.parent().unwrap().to_string_lossy().as_ref()),
        "stderr: {stderr}"
    );
}

#[test]
fn migrate_rejects_a_companion_from_another_release() {
    let pair = InstalledPair::with_companion();
    let output = pair.run(
        "999.0.0",
        0,
        &["migrate", "v1", "status", "--manifest", "manifest.json"],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release mismatch"), "stderr: {stderr}");
    assert!(stderr.contains("999.0.0"), "stderr: {stderr}");
    assert!(!pair.capture.exists(), "operation must not be forwarded");
}

#[test]
fn migrate_rejects_a_companion_from_a_group_writable_install_directory() {
    let pair = InstalledPair::with_companion();
    set_mode(pair.reborn.parent().unwrap(), 0o775);
    let output = pair.run(
        env!("CARGO_PKG_VERSION"),
        0,
        &["migrate", "v1", "status", "--manifest", "manifest.json"],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("installation directory") && stderr.contains("writable by another user"),
        "stderr: {stderr}"
    );
    assert!(!pair.capture.exists(), "operation must not be forwarded");
}

#[test]
fn migrate_forwards_only_redacted_plan_options_and_propagates_exit_status() {
    let pair = InstalledPair::with_companion();
    let snapshot = pair._temp.path().join("v1-snapshot.db");
    let manifest = pair._temp.path().join("migration.json");
    let output = Command::new(&pair.reborn)
        .args([
            "migrate",
            "v1",
            "plan",
            "--source-libsql",
            snapshot.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--strict",
        ])
        .current_dir(pair._temp.path())
        .env_clear()
        .env("HOME", pair._temp.path().join("home"))
        .env("MIGRATION_TEST_CAPTURE", &pair.capture)
        .env("MIGRATION_TEST_EXIT_CODE", "23")
        .env(
            "MIGRATION_TEST_HANDSHAKE",
            handshake_document(env!("CARGO_PKG_VERSION")),
        )
        .env(
            "MIGRATION_SOURCE_POSTGRES",
            "postgres://user:secret@example.invalid/db",
        )
        .env("MIGRATION_SOURCE_SECRET_MASTER_KEY", "source-secret")
        .env("MIGRATION_TARGET_SECRET_MASTER_KEY", "target-secret")
        .output()
        .expect("run Reborn CLI");

    assert_eq!(output.status.code(), Some(23));
    let forwarded = std::fs::read_to_string(&pair.capture).expect("captured args");
    assert_eq!(
        forwarded.lines().collect::<Vec<_>>(),
        vec![
            "v1",
            "plan",
            "--source-libsql",
            snapshot.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
            "--strict",
        ]
    );
    assert!(!forwarded.contains("postgres://"));
    assert!(!forwarded.contains("source-secret"));
    assert!(!forwarded.contains("target-secret"));
}

#[test]
fn migrate_apply_requires_the_snapshot_confirmation_before_launch() {
    let pair = InstalledPair::with_companion();
    let output = pair.run(
        env!("CARGO_PKG_VERSION"),
        0,
        &[
            "migrate",
            "v1",
            "apply",
            "--source-libsql",
            "snapshot.db",
            "--plan",
            "manifest.json",
            "--confirm-v1-stopped",
        ],
    );

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--confirm-source-snapshot"),
        "stderr: {stderr}"
    );
    assert!(!pair.capture.exists(), "operation must not be forwarded");
}

#[test]
fn onboard_migrate_v1_invokes_plan_only_after_an_explicit_flag() {
    let pair = InstalledPair::with_companion();
    let v1_home = pair._temp.path().join("v1-home");
    let reborn_home = pair._temp.path().join("reborn-home");
    std::fs::create_dir(&v1_home).expect("v1 home");
    let source = v1_home.join("ironclaw.db");
    std::fs::write(&source, b"snapshot evidence").expect("source evidence");

    let output = Command::new(&pair.reborn)
        .args(["onboard", "--migrate-v1"])
        .current_dir(pair._temp.path())
        .env_clear()
        .env("HOME", pair._temp.path().join("home"))
        .env("IRONCLAW_BASE_DIR", &v1_home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("MIGRATION_TEST_CAPTURE", &pair.capture)
        .env("MIGRATION_TEST_EXIT_CODE", "0")
        .env(
            "MIGRATION_TEST_HANDSHAKE",
            handshake_document(env!("CARGO_PKG_VERSION")),
        )
        .output()
        .expect("run onboarding");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let forwarded = std::fs::read_to_string(&pair.capture).expect("captured args");
    assert_eq!(
        forwarded.lines().collect::<Vec<_>>(),
        vec![
            "v1",
            "plan",
            "--source-libsql",
            source.to_str().unwrap(),
            "--manifest",
            reborn_home
                .join("v1-migration-manifest.json")
                .to_str()
                .unwrap(),
        ]
    );
    let marker = std::fs::read_to_string(reborn_home.join(".onboard-completed.json"))
        .expect("onboarding marker");
    let marker: serde_json::Value = serde_json::from_str(&marker).expect("marker JSON");
    assert_eq!(marker["v1_migration"]["state"], "planned");
}
