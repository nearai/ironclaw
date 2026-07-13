use std::process::Command;

#[cfg(feature = "libsql")]
use std::path::Path;

fn companion_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ironclaw-reborn-migration")
}

#[test]
fn handshake_is_exact_and_machine_readable() {
    let output = Command::new(companion_bin())
        .arg("__handshake")
        .env_clear()
        .output()
        .expect("run migration companion");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let handshake: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("handshake JSON");
    assert_eq!(
        handshake["schema_version"],
        "ironclaw.reborn.migration-companion/v1"
    );
    assert_eq!(
        handshake["protocol_version"],
        ironclaw_reborn_migration::MIGRATION_PROTOCOL_VERSION
    );
    assert_eq!(handshake["release_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn lifecycle_help_never_accepts_raw_postgres_urls_or_keys() {
    let output = Command::new(companion_bin())
        .args(["v1", "plan", "--help"])
        .env_clear()
        .output()
        .expect("run migration companion help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--source-postgres"), "stdout: {stdout}");
    assert!(stdout.contains("--source-home"), "stdout: {stdout}");
    assert!(
        !stdout.contains("--source-postgres-url"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("--target-postgres"), "stdout: {stdout}");
    assert!(!stdout.contains("--secret-master-key"), "stdout: {stdout}");
}

#[test]
fn companion_can_emit_a_machine_readable_error_envelope() {
    let output = Command::new(companion_bin())
        .args(["v1", "status", "--manifest", "/missing/manifest.json"])
        .env_clear()
        .env("IRONCLAW_REBORN_MIGRATION_ERROR_FORMAT", "json")
        .output()
        .expect("run migration companion");

    assert!(!output.status.success());
    let error: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("machine-readable error JSON");
    assert_eq!(
        error["schema_version"],
        "ironclaw.reborn.migration-error/v1"
    );
    assert_eq!(error["code"], "migration_failed");
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("failed to read migration manifest")),
        "error: {error}"
    );
}

#[cfg(feature = "libsql")]
async fn seed_empty_v1_source(path: &Path) {
    let database = libsql::Builder::new_local(path)
        .build()
        .await
        .expect("build source");
    let connection = database.connect().expect("connect source");
    connection
        .execute_batch(
            "CREATE TABLE settings (
                user_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY (user_id, key)
             );",
        )
        .await
        .expect("seed empty source");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn strict_plan_ignores_empty_lossy_categories() {
    let directory = tempfile::tempdir().expect("tempdir");
    let source_home = directory.path().join("v1-home");
    let reborn_home = directory.path().join("reborn-home");
    let source = source_home.join("ironclaw.db");
    let manifest = directory.path().join("migration.json");
    std::fs::create_dir_all(&source_home).expect("create source home");
    seed_empty_v1_source(&source).await;

    let output = Command::new(companion_bin())
        .args([
            "v1",
            "plan",
            "--source-libsql",
            source.to_str().expect("UTF-8 source path"),
            "--source-home",
            source_home.to_str().expect("UTF-8 source home"),
            "--manifest",
            manifest.to_str().expect("UTF-8 manifest path"),
            "--strict",
        ])
        .env_clear()
        .env("HOME", directory.path())
        .env("IRONCLAW_REBORN_HOME", reborn_home)
        .env("IRONCLAW_REBORN_PROFILE", "local-dev")
        .output()
        .expect("run strict migration plan");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        manifest.exists(),
        "strict plan should still write its manifest"
    );
}
