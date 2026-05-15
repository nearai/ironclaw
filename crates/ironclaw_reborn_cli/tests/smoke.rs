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
fn run_initializes_minimal_runtime_shell_without_touching_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home_dir = temp.path().join("home");
    let v1_base_dir = temp.path().join("v1-state");

    let output = Command::new(reborn_bin())
        .arg("run")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", &home_dir)
        .env("IRONCLAW_BASE_DIR", &v1_base_dir)
        .env_remove("USERPROFILE")
        .env_remove("IRONCLAW_REBORN_PROFILE")
        // Inherited TELEGRAM_BOT_TOKEN would push `run` into the Telegram
        // v2 serve loop, which is not what this test exercises.
        .env_remove("TELEGRAM_BOT_TOKEN")
        .output()
        .expect("ironclaw-reborn run should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn runtime shell"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(reborn_home.to_str().expect("utf8 path")),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("profile: local-dev"), "stdout: {stdout}");
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
    assert!(
        stdout.contains("runtime_shell: initialized"),
        "stdout: {stdout}"
    );
    assert!(
        !reborn_home.exists(),
        "minimal runtime shell should not create Reborn state directories"
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

/// Regression: when `TELEGRAM_BOT_TOKEN` is set, `run` must enter the
/// Telegram v2 serve path — *not* fall through to the runtime-shell
/// snapshot. We exercise this by setting the token but withholding the
/// webhook secret, which forces `HostConfig::from_env` to fail before
/// any bind happens. The error proves we crossed into the host crate;
/// the absence of the runtime-shell banner proves the fallback didn't
/// fire. Caller-level coverage per `.claude/rules/testing.md` —
/// `telegram_v2_configured_in_env` gates a side-effecting boot, and a
/// unit test on the helper alone wouldn't catch a missing wire-up in
/// `commands/run.rs`.
#[test]
#[cfg(feature = "telegram-v2")]
fn run_enters_telegram_v2_path_when_bot_token_is_set() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home_dir = temp.path().join("home");

    let output = Command::new(reborn_bin())
        .arg("run")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", &home_dir)
        .env_remove("USERPROFILE")
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .env("TELEGRAM_BOT_TOKEN", "fake-token-for-test")
        .env_remove("TELEGRAM_WEBHOOK_SECRET")
        .env_remove("LIBSQL_PATH")
        .env_remove("DATABASE_URL")
        .env_remove("IRONCLAW_REBORN_ALLOW_EPHEMERAL")
        .output()
        .expect("ironclaw-reborn run should run");

    assert!(
        !output.status.success(),
        "expected non-zero exit when Telegram v2 env is incomplete"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("TELEGRAM_WEBHOOK_SECRET") || stderr.contains("configuration error"),
        "stderr should report Telegram v2 config failure, got: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("IronClaw Reborn runtime shell"),
        "runtime-shell fallback must not fire once Telegram v2 path is taken; stdout: {stdout}"
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
