use std::{
    io::Write,
    process::{Command, Stdio},
};

const INVALID_PROFILE_MESSAGE: &str = "IRONCLAW_REBORN_PROFILE must be one of";

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
    assert!(stdout.contains("channels"), "stdout: {stdout}");
    assert!(stdout.contains("completion"), "stdout: {stdout}");
    assert!(stdout.contains("config"), "stdout: {stdout}");
    assert!(stdout.contains("doctor"), "stdout: {stdout}");
    assert!(stdout.contains("hooks"), "stdout: {stdout}");
    assert!(stdout.contains("logs"), "stdout: {stdout}");
    assert!(stdout.contains("models"), "stdout: {stdout}");
    assert!(stdout.contains("profile"), "stdout: {stdout}");
    assert!(stdout.contains("run"), "stdout: {stdout}");
    assert!(stdout.contains("skills"), "stdout: {stdout}");
}

#[test]
fn profile_list_shows_supported_profiles_without_reborn_home() {
    let output = Command::new(reborn_bin())
        .arg("profile")
        .arg("list")
        .env_clear()
        .output()
        .expect("ironclaw-reborn profile list should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn profiles"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("local-dev (default)"), "stdout: {stdout}");
    assert!(stdout.contains("production"), "stdout: {stdout}");
    assert!(stdout.contains("migration-dry-run"), "stdout: {stdout}");
    assert!(
        stdout.contains("IRONCLAW_REBORN_PROFILE"),
        "stdout: {stdout}"
    );
}

#[test]
fn profile_list_json_is_stable_and_does_not_resolve_reborn_home() {
    let output = Command::new(reborn_bin())
        .arg("profile")
        .arg("list")
        .arg("--json")
        .env_clear()
        .output()
        .expect("ironclaw-reborn profile list --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(json["selector"], "IRONCLAW_REBORN_PROFILE");
    let profiles = json["profiles"].as_array().expect("profiles array");
    assert_eq!(profiles.len(), 3);
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "local-dev" && profile["default"] == true)
    );
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "production" && profile["default"] == false)
    );
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "migration-dry-run" && profile["default"] == false)
    );
}

#[test]
fn channels_list_reports_unwired_empty_surface_without_reborn_home() {
    assert_empty_not_wired_surface(
        &["channels", "list"],
        "IronClaw Reborn channels",
        "channels",
        "configured",
    );
}

#[test]
fn channels_list_verbose_explains_missing_reborn_registry() {
    assert_verbose_detail(
        &["channels", "list", "--verbose"],
        "Reborn channel registry is not wired yet",
    );
}

#[test]
fn channels_list_json_verbose_includes_status_details() {
    assert_json_verbose_detail(
        &["channels", "list", "--json", "--verbose"],
        "channels",
        "configured",
        "Reborn channel registry is not wired yet",
    );
}

#[test]
fn hooks_list_reports_unwired_empty_surface_without_reborn_home() {
    assert_empty_not_wired_surface(
        &["hooks", "list"],
        "IronClaw Reborn hooks",
        "hooks",
        "configured",
    );
}

#[test]
fn hooks_list_verbose_explains_missing_reborn_registry() {
    assert_verbose_detail(
        &["hooks", "list", "--verbose"],
        "Reborn hook registry is not wired yet",
    );
}

#[test]
fn hooks_list_json_verbose_includes_status_details() {
    assert_json_verbose_detail(
        &["hooks", "list", "--json", "--verbose"],
        "hooks",
        "configured",
        "Reborn hook registry is not wired yet",
    );
}

#[test]
fn skills_list_reports_unwired_empty_surface_without_reborn_home() {
    assert_empty_not_wired_surface(
        &["skills", "list"],
        "IronClaw Reborn skills",
        "skills",
        "configured",
    );
}

#[test]
fn skills_list_verbose_explains_missing_reborn_catalog() {
    assert_verbose_detail(
        &["skills", "list", "--verbose"],
        "Reborn skill catalog is not wired yet",
    );
}

#[test]
fn skills_list_json_verbose_includes_status_details() {
    assert_json_verbose_detail(
        &["skills", "list", "--json", "--verbose"],
        "skills",
        "configured",
        "Reborn skill catalog is not wired yet",
    );
}

#[test]
fn logs_reports_unwired_surface_without_reborn_home() {
    assert_empty_not_wired_surface(&["logs"], "IronClaw Reborn logs", "logs", "entries");
}

#[test]
fn logs_verbose_explains_missing_reborn_log_source() {
    assert_verbose_detail(&["logs", "--verbose"], "Reborn log source is not wired yet");
}

#[test]
fn logs_json_verbose_includes_status_details() {
    assert_json_verbose_detail(
        &["logs", "--json", "--verbose"],
        "logs",
        "entries",
        "Reborn log source is not wired yet",
    );
}

#[test]
fn models_list_reports_reborn_slots_without_reborn_home() {
    let output = Command::new(reborn_bin())
        .arg("models")
        .arg("list")
        .env_clear()
        .output()
        .expect("ironclaw-reborn models list should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn model slots"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("- default"), "stdout: {stdout}");
    assert!(stdout.contains("- mission"), "stdout: {stdout}");
    assert!(
        stdout.contains("routes: not-configured"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
}

#[test]
fn models_status_json_reports_routes_not_configured() {
    let output = Command::new(reborn_bin())
        .arg("models")
        .arg("status")
        .arg("--json")
        .env_clear()
        .output()
        .expect("ironclaw-reborn models status --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(json["routes"], "not-configured");
    assert_eq!(json["slots"]["default"], "not-configured");
    assert_eq!(json["slots"]["mission"], "not-configured");
    assert_eq!(json["v1_state"], "not-used");
}

fn assert_empty_not_wired_surface(
    args: &[&str],
    title: &str,
    collection_key: &str,
    count_key: &str,
) {
    let output = Command::new(reborn_bin())
        .args(args)
        .env_clear()
        .output()
        .expect("ironclaw-reborn command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(title), "stdout: {stdout}");
    assert!(
        stdout.contains(&format!("{count_key}: 0")),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("status: not-wired"), "stdout: {stdout}");
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");

    let mut json_args = args.to_vec();
    json_args.push("--json");
    let output = Command::new(reborn_bin())
        .args(json_args)
        .env_clear()
        .output()
        .expect("ironclaw-reborn JSON command should run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(json[count_key], 0);
    assert_eq!(
        json[collection_key]
            .as_array()
            .expect("collection array")
            .len(),
        0
    );
    assert_eq!(json["status"], "not-wired");
    assert_eq!(json["v1_state"], "not-used");
}

fn assert_verbose_detail(args: &[&str], expected_detail: &str) {
    let output = Command::new(reborn_bin())
        .args(args)
        .env_clear()
        .output()
        .expect("ironclaw-reborn verbose command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(expected_detail), "stdout: {stdout}");
}

fn assert_json_verbose_detail(
    args: &[&str],
    collection_key: &str,
    count_key: &str,
    expected_detail: &str,
) {
    let output = Command::new(reborn_bin())
        .args(args)
        .env_clear()
        .output()
        .expect("ironclaw-reborn JSON verbose command should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(json[count_key], 0);
    assert_eq!(
        json[collection_key]
            .as_array()
            .expect("collection array")
            .len(),
        0
    );
    let details = json["details"].as_array().expect("details array");
    assert!(
        details.iter().any(|detail| detail == expected_detail),
        "json: {json}"
    );
}

#[test]
fn config_path_reports_reborn_home_without_touching_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let v1_base_dir = temp.path().join("v1-state");

    let output = Command::new(reborn_bin())
        .arg("config")
        .arg("path")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_REBORN_PROFILE", "production")
        .env("IRONCLAW_BASE_DIR", &v1_base_dir)
        .output()
        .expect("ironclaw-reborn config path should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn config path"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(&format!("reborn_home: {}", reborn_home.display())),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("home_source: IRONCLAW_REBORN_HOME"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("profile: production"), "stdout: {stdout}");
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
    assert!(
        !reborn_home.exists(),
        "config path should not create Reborn state directories"
    );
    assert!(
        !v1_base_dir.exists(),
        "config path should not create explicit v1 base directories"
    );
}

#[test]
fn config_path_reports_default_reborn_home_without_creating_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join(".ironclaw").join("reborn");

    let output = Command::new(reborn_bin())
        .arg("config")
        .arg("path")
        .env_remove("IRONCLAW_REBORN_HOME")
        .env("HOME", temp.path())
        .env_remove("USERPROFILE")
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn config path should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("reborn_home: {}", reborn_home.display())),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("home_source: default"), "stdout: {stdout}");
    assert!(stdout.contains("profile: local-dev"), "stdout: {stdout}");
    assert!(
        !temp.path().join(".ironclaw").exists(),
        "config path should not create default Reborn or v1 state directories"
    );
}

#[test]
fn completion_generates_zsh_script_without_reborn_home() {
    let output = Command::new(reborn_bin())
        .arg("completion")
        .arg("--shell")
        .arg("zsh")
        .env_clear()
        .output()
        .expect("ironclaw-reborn completion should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("#compdef ironclaw-reborn"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("_ironclaw-reborn"), "stdout: {stdout}");
    assert!(
        stdout.contains("$+functions[compdef]"),
        "zsh completion should guard compdef: {stdout}"
    );
}

#[test]
fn completion_generates_bash_script_without_reborn_home() {
    let output = Command::new(reborn_bin())
        .arg("completion")
        .arg("--shell")
        .arg("bash")
        .env_clear()
        .output()
        .expect("ironclaw-reborn completion should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("_ironclaw-reborn()"), "stdout: {stdout}");
    assert!(stdout.contains("COMPREPLY"), "stdout: {stdout}");
}

#[test]
fn run_reports_runtime_readiness_snapshot_without_touching_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home_dir = temp.path().join("home");
    let v1_base_dir = temp.path().join("v1-state");

    // `--dry-run` preserves the legacy diagnostic-only behavior: no agent
    // is started, no state directories are created. The same shell
    // identifiers (profile, home, v1_state, readiness) are reported so
    // existing tooling that scrapes `run` output keeps working. Without
    // the flag, `run` boots the live agent and would create the local-dev
    // root, which the rest of this test forbids.
    let output = Command::new(reborn_bin())
        .arg("run")
        .arg("--dry-run")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", &home_dir)
        .env("IRONCLAW_BASE_DIR", &v1_base_dir)
        .env_remove("USERPROFILE")
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn run should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn runtime readiness snapshot"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(reborn_home.to_str().expect("utf8 path")),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("profile: local-dev"), "stdout: {stdout}");
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
    assert!(
        stdout.contains("runtime_driver: planned-agent-loop"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("local_runtime_shell_readiness: ready"),
        "stdout: {stdout}"
    );
    assert!(
        !reborn_home.exists(),
        "runtime readiness snapshot should not create Reborn state directories"
    );
    assert!(
        !home_dir.join(".ironclaw").exists(),
        "minimal runtime shell should not create default v1 state directories"
    );
    assert!(
        !v1_base_dir.exists(),
        "minimal runtime shell should not create explicit v1 base directories"
    );
}

#[test]
fn doctor_uses_reborn_home_override_without_touching_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
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
    assert!(stdout.contains("profile: local-dev"), "stdout: {stdout}");
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
    assert!(
        stdout.contains("driver_registry: initialized"),
        "stdout: {stdout}"
    );
    assert!(
        !reborn_home.exists(),
        "doctor should not create state directories"
    );
}

#[test]
fn run_message_exits_nonzero_when_runtime_does_not_produce_reply() {
    let temp = tempfile::tempdir().expect("tempdir");

    let output = Command::new(reborn_bin())
        .arg("run")
        .arg("--message")
        .arg("hello")
        .env_clear()
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env("HOME", temp.path().join("home"))
        .output()
        .expect("ironclaw-reborn run --message should run");

    assert!(
        !output.status.success(),
        "run --message should fail when the runtime cannot produce assistant text"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout should stay reply-only: {stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("reborn run did not produce an assistant reply"),
        "stderr: {stderr}"
    );
}

#[test]
fn run_piped_stdin_exits_nonzero_when_runtime_does_not_produce_reply() {
    let temp = tempfile::tempdir().expect("tempdir");

    let mut child = Command::new(reborn_bin())
        .arg("run")
        .env_clear()
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env("HOME", temp.path().join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn run should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"  hello  \n")
        .expect("prompt should be written");
    let output = child
        .wait_with_output()
        .expect("ironclaw-reborn run should finish");

    assert!(
        !output.status.success(),
        "piped run should fail when the runtime cannot produce assistant text"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout should stay reply-only: {stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("reborn run did not produce an assistant reply"),
        "stderr: {stderr}"
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
        .env_remove("IRONCLAW_REBORN_PROFILE")
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
    assert!(stdout.contains("profile: local-dev"), "stdout: {stdout}");
    assert!(
        !temp.path().join(".ironclaw").exists(),
        "doctor should not create default Reborn or v1 state directories"
    );
}

#[test]
fn doctor_reports_explicit_profile() {
    let temp = tempfile::tempdir().expect("tempdir");

    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env("IRONCLAW_REBORN_PROFILE", "production")
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("profile: production"), "stdout: {stdout}");
}

#[test]
fn run_reports_explicit_profile() {
    let temp = tempfile::tempdir().expect("tempdir");

    // Production / migration-dry-run profiles are recognized by the boot
    // config but not yet wired into the assembled runtime. `--dry-run`
    // exercises the boot-config path without booting the agent.
    let output = Command::new(reborn_bin())
        .arg("run")
        .arg("--dry-run")
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env("IRONCLAW_REBORN_PROFILE", "migration-dry-run")
        .output()
        .expect("ironclaw-reborn run should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("profile: migration-dry-run"),
        "stdout: {stdout}"
    );
}

#[test]
fn doctor_rejects_invalid_profile() {
    let temp = tempfile::tempdir().expect("tempdir");

    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env("IRONCLAW_REBORN_PROFILE", "prod")
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(
        !output.status.success(),
        "doctor should reject invalid profile"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(INVALID_PROFILE_MESSAGE), "stderr: {stderr}");
}

#[test]
fn doctor_rejects_empty_profile_override() {
    let temp = tempfile::tempdir().expect("tempdir");

    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env("IRONCLAW_REBORN_PROFILE", "")
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(
        !output.status.success(),
        "doctor should reject empty profile"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(INVALID_PROFILE_MESSAGE), "stderr: {stderr}");
}

#[test]
fn run_rejects_invalid_profile() {
    let temp = tempfile::tempdir().expect("tempdir");

    let output = Command::new(reborn_bin())
        .arg("run")
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env("IRONCLAW_REBORN_PROFILE", "prod")
        .output()
        .expect("ironclaw-reborn run should run");

    assert!(
        !output.status.success(),
        "run should reject invalid profile"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(INVALID_PROFILE_MESSAGE), "stderr: {stderr}");
}

#[test]
fn run_rejects_reborn_home_equal_to_explicit_v1_base_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v1_root = temp.path().join("v1-state");

    let output = Command::new(reborn_bin())
        .arg("run")
        .env("IRONCLAW_REBORN_HOME", &v1_root)
        .env("IRONCLAW_BASE_DIR", &v1_root)
        .output()
        .expect("ironclaw-reborn run should run");

    assert!(!output.status.success(), "run should reject v1 root");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_HOME must not point at the v1 IronClaw state root"),
        "stderr: {stderr}"
    );
}

#[test]
fn doctor_rejects_reborn_home_equal_to_explicit_v1_base_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v1_root = temp.path().join("v1-state");

    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env("IRONCLAW_REBORN_HOME", &v1_root)
        .env("IRONCLAW_BASE_DIR", &v1_root)
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(!output.status.success(), "doctor should reject v1 root");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_HOME must not point at the v1 IronClaw state root"),
        "stderr: {stderr}"
    );
}

#[test]
fn doctor_rejects_reborn_home_equal_to_relative_explicit_v1_base_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v1_root = temp.path().join("v1-state");

    let output = Command::new(reborn_bin())
        .arg("doctor")
        .current_dir(temp.path())
        .env("IRONCLAW_REBORN_HOME", &v1_root)
        .env("IRONCLAW_BASE_DIR", "v1-state")
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(!output.status.success(), "doctor should reject v1 root");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_HOME must not point at the v1 IronClaw state root"),
        "stderr: {stderr}"
    );
}

#[test]
fn doctor_rejects_empty_reborn_home_override() {
    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env_clear()
        .env("IRONCLAW_REBORN_HOME", "")
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(!output.status.success(), "doctor should reject empty home");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_HOME must not be empty"),
        "stderr: {stderr}"
    );
}

#[test]
fn doctor_rejects_relative_reborn_home_override() {
    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env_clear()
        .env("IRONCLAW_REBORN_HOME", "relative/reborn")
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(
        !output.status.success(),
        "doctor should reject relative home"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_HOME must be an absolute path"),
        "stderr: {stderr}"
    );
}

#[test]
fn doctor_rejects_missing_home_for_default_reborn_home() {
    let output = Command::new(reborn_bin())
        .arg("doctor")
        .env_clear()
        .output()
        .expect("ironclaw-reborn doctor should run");

    assert!(
        !output.status.success(),
        "doctor should reject missing home"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("HOME or USERPROFILE must be set"),
        "stderr: {stderr}"
    );
}

// ─── Boot-config TOML + provider catalog (epic #3036 prep) ───────────────────

#[test]
fn config_init_writes_both_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let output = Command::new(reborn_bin())
        .args(["config", "init"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn config init should run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        reborn_home.join("config.toml").exists(),
        "config.toml missing"
    );
    assert!(
        reborn_home.join("providers.json").exists(),
        "providers.json missing"
    );
    let config_text =
        std::fs::read_to_string(reborn_home.join("config.toml")).expect("config.toml readable");
    assert!(
        config_text.contains("api_version = \"ironclaw.runtime/v1\""),
        "config.toml should stamp api_version; got: {config_text}"
    );
}

#[test]
fn config_init_refuses_to_clobber_without_force() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let first = Command::new(reborn_bin())
        .args(["config", "init"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("first init should run");
    assert!(first.status.success());

    let second = Command::new(reborn_bin())
        .args(["config", "init"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("second init should run");
    assert!(
        !second.status.success(),
        "second init must refuse to clobber"
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("already exists") && stderr.contains("--force"),
        "stderr should point at --force; got: {stderr}"
    );
}

#[test]
fn config_init_preflights_both_targets_before_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(reborn_home.join("providers.json"), "[]\n").expect("write providers");

    let output = Command::new(reborn_bin())
        .args(["config", "init"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("init should run");
    assert!(!output.status.success(), "init must refuse clobber");
    assert!(
        !reborn_home.join("config.toml").exists(),
        "config.toml must not be written after providers preflight fails"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("providers.json") && stderr.contains("--force"),
        "stderr should name existing target and --force; got: {stderr}"
    );
}

#[test]
fn config_init_with_force_overwrites() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(reborn_home.join("config.toml"), "partial config\n").expect("write config");
    std::fs::write(reborn_home.join("providers.json"), "partial providers\n")
        .expect("write providers");

    let output = Command::new(reborn_bin())
        .args(["config", "init", "--force"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("forced init should run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let config_text =
        std::fs::read_to_string(reborn_home.join("config.toml")).expect("config.toml readable");
    let providers_text = std::fs::read_to_string(reborn_home.join("providers.json"))
        .expect("providers.json readable");
    assert!(!config_text.contains("partial config"));
    assert!(!providers_text.contains("partial providers"));
    assert!(config_text.contains("api_version = \"ironclaw.runtime/v1\""));
    assert!(providers_text.contains("\"id\": \"acme-openrouter\""));
}

#[test]
fn config_path_reports_file_presence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    // Pre-init: files are absent.
    let absent_output = Command::new(reborn_bin())
        .args(["config", "path"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("config path runs without files");
    assert!(absent_output.status.success());
    let absent_stdout = String::from_utf8_lossy(&absent_output.stdout);
    assert!(
        absent_stdout.contains("config_file") && absent_stdout.contains("absent"),
        "stdout: {absent_stdout}"
    );

    // After init: files report present.
    let init_output = Command::new(reborn_bin())
        .args(["config", "init"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("init runs");
    assert!(init_output.status.success());

    let present_output = Command::new(reborn_bin())
        .args(["config", "path"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("config path runs after init");
    assert!(present_output.status.success());
    let present_stdout = String::from_utf8_lossy(&present_output.stdout);
    assert!(
        present_stdout.contains("config_file") && present_stdout.contains("present"),
        "stdout: {present_stdout}"
    );
    assert!(
        present_stdout.contains("providers") && present_stdout.contains("present"),
        "stdout: {present_stdout}"
    );
}

#[test]
fn run_with_inline_secret_in_config_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    let bad_config = r#"
[llm.default]
provider_id = "openai"
api_key_env = "sk-proj-1234567890abcdef12345678"
"#;
    std::fs::write(reborn_home.join("config.toml"), bad_config).expect("write bad config");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(
        !output.status.success(),
        "inline secret must cause failure; stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("inline secret") || stderr.contains("secret"),
        "stderr should mention inline secret rejection; got: {stderr}"
    );
}

#[test]
fn run_warns_when_falling_back_to_stub_gateway() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no LLM selection configured") && stderr.contains("Runs will fail"),
        "stderr should warn about degraded stub-gateway boot; got: {stderr}"
    );
}

#[test]
fn run_honors_boot_profile_from_config_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[boot]
profile = "production"
"#,
    )
    .expect("write config");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(
        !output.status.success(),
        "production profile should fail until wired; stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("profile=production"),
        "stderr should mention config-selected profile; got: {stderr}"
    );
}

#[test]
fn run_rejects_inline_secret_in_provider_id_without_echoing_value() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    let secret = "sk-proj-1234567890abcdef1234567890";
    std::fs::write(
        reborn_home.join("config.toml"),
        format!(
            r#"
[llm.default]
provider_id = " {secret} "
"#
        ),
    )
    .expect("write config");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(!output.status.success(), "inline secret must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("inline secret") || stderr.contains("secret"),
        "stderr should mention secret rejection; got: {stderr}"
    );
    assert!(
        !stderr.contains(secret),
        "stderr must not echo pasted secret; got: {stderr}"
    );
}

#[test]
fn run_rejects_unsupported_identity_scope_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[identity]
tenant = "acme"
default_owner = "operator"
"#,
    )
    .expect("write config");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(!output.status.success(), "unsupported identity must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[identity]") && stderr.contains("tenant") && stderr.contains("not wired"),
        "stderr should explain unsupported identity scope; got: {stderr}"
    );
}

#[test]
fn run_rejects_unsupported_policy_driver_and_harness_sections() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[policy]
default_approval_policy = "ask_always"
"#,
    )
    .expect("write config");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(!output.status.success(), "unsupported policy must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[policy]") && stderr.contains("not wired"),
        "stderr should explain unsupported section; got: {stderr}"
    );
}

#[test]
fn run_rejects_malformed_explicit_provider_overlay() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[llm.default]
provider_id = "openai"
"#,
    )
    .expect("write config");
    std::fs::write(reborn_home.join("providers.json"), "not json").expect("write providers");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(!output.status.success(), "malformed overlay must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("provider catalog") || stderr.contains("providers.json"),
        "stderr should explain provider catalog load failure; got: {stderr}"
    );
}

#[test]
fn run_rejects_empty_required_api_key_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[llm.default]
provider_id = "empty-key-provider"
"#,
    )
    .expect("write config");
    std::fs::write(
        reborn_home.join("providers.json"),
        r#"[
  {
    "id": "empty-key-provider",
    "protocol": "open_ai_completions",
    "api_key_env": "REBORN_TEST_EMPTY_KEY",
    "api_key_required": true,
    "model_env": "REBORN_TEST_MODEL",
    "default_model": "test-model",
    "description": "test provider"
  }
]
"#,
    )
    .expect("write providers");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("REBORN_TEST_EMPTY_KEY", "")
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(!output.status.success(), "empty API key must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("REBORN_TEST_EMPTY_KEY") && stderr.contains("requires API key env var"),
        "stderr should treat empty key as unset; got: {stderr}"
    );
}

#[test]
fn run_rejects_zero_runner_heartbeat_interval() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[runner]
heartbeat_interval_secs = 0
"#,
    )
    .expect("write config");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(
        !output.status.success(),
        "zero heartbeat interval must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("heartbeat_interval_secs") && stderr.contains("greater than 0"),
        "stderr should explain heartbeat interval rejection; got: {stderr}"
    );
}

#[test]
fn run_rejects_zero_runner_poll_interval() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[runner]
poll_interval_ms = 0
"#,
    )
    .expect("write config");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(!output.status.success(), "zero poll interval must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("poll_interval_ms") && stderr.contains("greater than 0"),
        "stderr should explain poll interval rejection; got: {stderr}"
    );
}

#[test]
fn run_resolves_provider_from_config_and_demands_api_key_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    let cfg = r#"
[llm.default]
provider_id = "openai"
model = "gpt-4o-mini"
api_key_env = "REBORN_TEST_UNSET_BC8F4D_KEY"
"#;
    std::fs::write(reborn_home.join("config.toml"), cfg).expect("write config");

    let output = Command::new(reborn_bin())
        .args(["run", "-m", "ping"])
        .env_remove("USERPROFILE")
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OLLAMA_BASE_URL")
        .env_remove("REBORN_TEST_UNSET_BC8F4D_KEY")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(
        !output.status.success(),
        "missing api key must fail; stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("REBORN_TEST_UNSET_BC8F4D_KEY"),
        "stderr should name the unset env var; got: {stderr}"
    );
}
