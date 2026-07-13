use std::process::Command;

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
