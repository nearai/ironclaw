use std::process::Command;

fn reborn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ironclaw-reborn")
}

#[test]
fn help_mentions_reborn_commands() {
    let output = Command::new(reborn_bin())
        .arg("--help")
        .output()
        .expect("ironclaw-reborn --help should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Standalone IronClaw Reborn runtime"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("doctor"), "stdout: {stdout}");
}

#[test]
fn doctor_uses_reborn_home_override_without_touching_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn doctor"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(reborn_home.to_str().expect("utf8 path")),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
    assert!(
        !reborn_home.exists(),
        "doctor should not create state directories"
    );
}

#[test]
fn doctor_default_home_is_reborn_scoped_and_dry_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join(".ironclaw").join("reborn");

    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env_remove("IRONCLAW_REBORN_HOME")
        .env("HOME", temp.path())
        .env_remove("USERPROFILE")
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(reborn_home.to_str().expect("utf8 path")),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("home_source: default"), "stdout: {stdout}");
    assert!(
        !temp.path().join(".ironclaw").exists(),
        "doctor should not create default Reborn or v1 state directories"
    );
}
