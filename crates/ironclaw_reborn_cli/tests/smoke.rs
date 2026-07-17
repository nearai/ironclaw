// arch-exempt: large_file, centralized CLI and Dockerfile smoke contracts, plan #6058
#[cfg(feature = "webui-v2-beta")]
use std::io::BufRead;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const INVALID_PROFILE_MESSAGE: &str = "IRONCLAW_REBORN_PROFILE must be one of";

fn reborn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ironclaw-reborn")
}

/// Shared builder for every real-binary spawn in this file: `Command::new(reborn_bin())`
/// with `env_clear()` and `IRONCLAW_DISABLE_OS_KEYCHAIN=1` already applied — the
/// suppression a spawned real binary needs the same way `cfg!(test)` suppresses
/// keychain access for in-process unit tests (see
/// `ironclaw_secrets::keychain::os_keychain_suppressed`). Callers chain further
/// `.env(...)`/`.arg(...)` calls on top; this only centralizes the two lines every
/// site needs regardless of what else it sets up.
fn reborn_command() -> Command {
    let mut command = Command::new(reborn_bin());
    command.env_clear().env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1");
    command
}

fn assert_stdout_file_action(stdout: &str, file_name: &str, action: &str) {
    let prefix = format!("{action}: ");
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with(&prefix) && line.ends_with(file_name)),
        "stdout should contain {action}: <path> ending in {file_name}: {stdout}"
    );
}

fn assert_stdout_labeled_action(stdout: &str, label: &str, action: &str) {
    let suffix = format!(" ({action})");
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with(label) && line.ends_with(&suffix)),
        "stdout should contain {label} with action {action}: {stdout}"
    );
}

fn isolated_no_llm_command(workspace: &Path, reborn_home: &Path) -> Command {
    let mut command = reborn_command();
    command
        .current_dir(workspace)
        .env("HOME", workspace.join("isolated-home"))
        .env("LLM_USE_CODEX_AUTH", "false")
        .env("LLM_BACKEND", "")
        .env("LLM_MODEL", "")
        .env("OPENAI_MODEL", "")
        .env("OPENAI_CODEX_MODEL", "")
        .env("OPENAI_API_KEY", "")
        .env("ANTHROPIC_API_KEY", "")
        .env("OLLAMA_BASE_URL", "")
        .env("IRONCLAW_REBORN_HOME", reborn_home);
    command
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[cfg(unix)]
fn fake_reborn_bin(bin_dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::create_dir_all(bin_dir).expect("fake bin dir");
    let bin = bin_dir.join("ironclaw-reborn");
    std::fs::write(
        &bin,
        "#!/bin/sh\nprintf 'home=%s\\n' \"$IRONCLAW_REBORN_HOME\"\nprintf 'args=%s\\n' \"$*\"\n",
    )
    .expect("write fake reborn bin");
    let mut permissions = std::fs::metadata(&bin)
        .expect("fake bin metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&bin, permissions).expect("chmod fake bin");
}

#[cfg(unix)]
fn fake_bin_path(bin_dir: &Path) -> String {
    format!("{}:/usr/bin:/bin", bin_dir.display())
}

#[cfg(unix)]
fn write_reborn_config(reborn_home: &Path, profile: &str) {
    std::fs::create_dir_all(reborn_home).expect("reborn home");
    let production_sections = match profile {
        "production" | "migration-dry-run" => {
            "\n[policy]\ndeployment_mode = \"hosted_multi_tenant\"\ndefault_profile = \"secure_default\"\n\n[storage]\nbackend = \"postgres\"\nurl_env = \"IRONCLAW_REBORN_POSTGRES_URL\"\nsecret_master_key_env = \"IRONCLAW_REBORN_SECRET_MASTER_KEY\"\n"
        }
        _ => "",
    };
    std::fs::write(
        reborn_home.join("config.toml"),
        format!(
            "api_version = \"ironclaw.runtime/v1\"\n\n[boot]\nprofile = \"{profile}\"\n{production_sections}"
        ),
    )
    .expect("config");
}

#[cfg(unix)]
fn write_sparse_reborn_config(reborn_home: &Path) {
    std::fs::create_dir_all(reborn_home).expect("reborn home");
    std::fs::write(
        reborn_home.join("config.toml"),
        "api_version = \"ironclaw.runtime/v1\"\n",
    )
    .expect("config");
}

#[test]
fn dockerfile_reborn_builds_with_postgres_feature() {
    let dockerfile = std::fs::read_to_string(workspace_root().join("Dockerfile.reborn"))
        .expect("Dockerfile.reborn");

    assert!(
        dockerfile
            .matches("webui-v2-beta,slack-v2-host-beta,libsql,postgres")
            .count()
            >= 2,
        "Dockerfile.reborn must compile both cargo-chef deps and final binary with libsql and postgres: {dockerfile}"
    );
    assert!(
        dockerfile.contains("corepack enable pnpm")
            && dockerfile.matches("pnpm install --frozen-lockfile").count() >= 2
            && dockerfile.contains("crates/ironclaw_webui/frontend"),
        "Dockerfile.reborn must install WebUI frontend dependencies before cargo-chef and final webui-v2-beta builds: {dockerfile}"
    );
    assert!(
        dockerfile.contains("config.production.toml"),
        "Dockerfile.reborn must ship the opt-in production config: {dockerfile}"
    );
    assert!(
        dockerfile.contains("config.hosted-single-tenant-volume.toml"),
        "Dockerfile.reborn must ship the hosted volume seed config: {dockerfile}"
    );
    let builder_stage = dockerfile
        .split_once("FROM deps AS builder")
        .map(|(_, stage)| stage)
        .expect("Dockerfile.reborn should define a builder stage");
    assert!(
        builder_stage.contains("COPY migrations/ migrations/")
            && dockerfile.matches("COPY migrations/ migrations/").count() == 1,
        "Dockerfile.reborn must copy repo-level SQL migrations exactly once in the builder stage for postgres include_str! builds: {dockerfile}"
    );
    assert!(
        !dockerfile.contains("IRONCLAW_REBORN_HOME=/data/ironclaw-reborn"),
        "Dockerfile.reborn must let the entrypoint resolve Railway volume mounts before falling back to /data: {dockerfile}"
    );
    assert!(
        !dockerfile.contains("\nVOLUME "),
        "Railway's Dockerfile builder rejects Docker VOLUME instructions; configure Railway volumes outside the image: {dockerfile}"
    );
}

#[test]
fn dockerfile_reborn_ships_extension_ownership_migration() {
    let dockerfile = std::fs::read_to_string(workspace_root().join("Dockerfile.reborn"))
        .expect("Dockerfile.reborn");
    let deps_stage = dockerfile
        .split_once("FROM chef AS deps")
        .and_then(|(_, stages)| stages.split_once("FROM deps AS builder"))
        .map(|(stage, _)| stage)
        .expect("Dockerfile.reborn should define a deps stage");
    let builder_stage = dockerfile
        .split_once("FROM deps AS builder")
        .map(|(_, stage)| stage)
        .expect("Dockerfile.reborn should define a builder stage");

    assert!(
        deps_stage.contains("--package ironclaw_reborn_migration")
            && deps_stage.contains("--no-default-features")
            && deps_stage.contains("--features libsql")
            && deps_stage.contains("--recipe-path recipe.json"),
        "Dockerfile.reborn must cache the libSQL-only extension ownership migration dependencies: {dockerfile}"
    );
    assert!(
        builder_stage.contains("--package ironclaw_reborn_migration")
            && builder_stage.contains("--no-default-features")
            && builder_stage.contains("--features libsql")
            && builder_stage.contains("--bin ironclaw-reborn-extension-ownership-migration"),
        "Dockerfile.reborn must build the libSQL-only extension ownership migration binary: {dockerfile}"
    );
    assert!(
        dockerfile.contains(
            "COPY --from=builder /app/target/dist/ironclaw-reborn-extension-ownership-migration /usr/local/bin/ironclaw-reborn-extension-ownership-migration"
        ),
        "Dockerfile.reborn must copy the extension ownership migration into the runtime image: {dockerfile}"
    );
}

#[test]
fn run_reborn_webui_builds_frontend_before_cargo() {
    let launcher = std::fs::read_to_string(workspace_root().join("scripts/run-reborn-webui.sh"))
        .expect("scripts/run-reborn-webui.sh");

    let frontend_build = launcher
        .find("pnpm build")
        .expect("launcher should build WebUI frontend assets");
    let cargo_run = launcher
        .find("CARGO=(cargo run -q -p ironclaw_reborn_cli --features webui-v2-beta")
        .expect("launcher should run Reborn with webui-v2-beta");
    assert!(
        frontend_build < cargo_run,
        "scripts/run-reborn-webui.sh must build frontend/dist before cargo compiles webui-v2-beta: {launcher}"
    );
}

#[test]
fn docker_reborn_config_defaults_to_local_dev() {
    let config = std::fs::read_to_string(workspace_root().join("docker/reborn/config.toml"))
        .expect("docker reborn config");
    let parsed = ironclaw_reborn_config::RebornConfigFile::parse_text(
        &config,
        &workspace_root().join("docker/reborn/config.toml"),
    )
    .expect("docker reborn config parses");

    let boot = parsed.boot.expect("docker config must have [boot]");
    assert_eq!(boot.profile.as_deref(), Some("local-dev"));
    assert!(
        parsed.storage.is_none(),
        "local Docker config must not require production storage"
    );
    assert!(
        parsed.policy.is_none(),
        "local Docker config must not include production-only policy"
    );
}

#[test]
fn docker_reborn_production_config_uses_postgres_storage() {
    let config =
        std::fs::read_to_string(workspace_root().join("docker/reborn/config.production.toml"))
            .expect("docker reborn production config");
    let parsed = ironclaw_reborn_config::RebornConfigFile::parse_text(
        &config,
        &workspace_root().join("docker/reborn/config.production.toml"),
    )
    .expect("docker reborn production config parses");

    let boot = parsed
        .boot
        .expect("docker production config must have [boot]");
    assert_eq!(boot.profile.as_deref(), Some("production"));

    let storage = parsed.storage.expect("docker config must have [storage]");
    assert_eq!(
        storage.backend,
        Some(ironclaw_reborn_config::StorageBackend::Postgres)
    );
    assert_eq!(
        storage.url_env.as_deref(),
        Some("IRONCLAW_REBORN_POSTGRES_URL")
    );
    assert_eq!(
        storage.secret_master_key_env.as_deref(),
        Some("IRONCLAW_REBORN_SECRET_MASTER_KEY")
    );
    assert_eq!(storage.pool_max_size, Some(2));

    let policy = parsed
        .policy
        .expect("docker config must provide the production runtime policy required by #4645");
    assert_eq!(
        policy.deployment_mode.as_deref(),
        Some("hosted_multi_tenant")
    );
    assert_eq!(policy.default_profile.as_deref(), Some("secure_default"));
}

#[cfg(unix)]
#[test]
fn docker_reborn_entrypoint_uses_railway_volume_mount_for_home() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fake_reborn_bin(&bin_dir);
    let volume = temp.path().join("railway-volume");
    let reborn_home = volume.join("ironclaw-reborn");
    write_reborn_config(&reborn_home, "local-dev");

    let output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .arg("--help")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("RAILWAY_ENVIRONMENT", "production")
        .env("RAILWAY_VOLUME_MOUNT_PATH", &volume)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("home={}", reborn_home.display())),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("args=--help"), "stdout: {stdout}");
}

#[cfg(unix)]
#[test]
fn docker_reborn_entrypoint_rejects_ephemeral_railway_without_volume() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fake_reborn_bin(&bin_dir);
    let reborn_home = temp.path().join("reborn-home");
    write_reborn_config(&reborn_home, "local-dev");

    let output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("RAILWAY_ENVIRONMENT", "production")
        .output()
        .expect("entrypoint should run");

    assert!(!output.status.success(), "entrypoint should fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Railway deployment using profile=local-dev requires a persistent volume"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY=true"),
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn docker_reborn_entrypoint_rejects_sparse_config_as_local_dev_on_railway() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fake_reborn_bin(&bin_dir);
    let reborn_home = temp.path().join("reborn-home");
    write_sparse_reborn_config(&reborn_home);

    let output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("RAILWAY_ENVIRONMENT", "production")
        .output()
        .expect("entrypoint should run");

    assert!(!output.status.success(), "entrypoint should fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Railway deployment using profile=local-dev requires a persistent volume"),
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn docker_reborn_entrypoint_rejects_local_dev_home_outside_railway_volume() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fake_reborn_bin(&bin_dir);
    let volume = temp.path().join("railway-volume");
    let reborn_home = temp.path().join("ephemeral-home");
    write_reborn_config(&reborn_home, "local-dev");

    let output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .arg("--help")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("RAILWAY_ENVIRONMENT", "production")
        .env("RAILWAY_VOLUME_MOUNT_PATH", &volume)
        .output()
        .expect("entrypoint should run");

    assert!(!output.status.success(), "entrypoint should fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("to be under RAILWAY_VOLUME_MOUNT_PATH"),
        "stderr: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn docker_reborn_entrypoint_allows_railway_production_without_volume() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fake_reborn_bin(&bin_dir);
    let reborn_home = temp.path().join("reborn-home");
    write_reborn_config(&reborn_home, "production");

    let output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .arg("--help")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_REBORN_PROFILE", "production")
        .env("RAILWAY_ENVIRONMENT", "production")
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("home={}", reborn_home.display())),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("args=--help"), "stdout: {stdout}");
}

#[cfg(unix)]
#[test]
fn docker_reborn_entrypoint_rejects_stale_local_dev_config_for_production() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fake_reborn_bin(&bin_dir);
    let reborn_home = temp.path().join("reborn-home");
    write_reborn_config(&reborn_home, "local-dev");

    let output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .arg("--help")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_REBORN_PROFILE", "production")
        .env("RAILWAY_ENVIRONMENT", "production")
        .output()
        .expect("entrypoint should run");

    assert!(!output.status.success(), "entrypoint should fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_PROFILE=production requires"),
        "stderr: {stderr}"
    );
    assert!(stderr.contains("stale local-dev seed"), "stderr: {stderr}");
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
    assert!(stdout.contains("extension"), "stdout: {stdout}");
    assert!(stdout.contains("hooks"), "stdout: {stdout}");
    assert!(stdout.contains("logs"), "stdout: {stdout}");
    assert!(stdout.contains("models"), "stdout: {stdout}");
    assert!(stdout.contains("onboard"), "stdout: {stdout}");
    assert!(stdout.contains("profile"), "stdout: {stdout}");
    assert!(stdout.contains("repl"), "stdout: {stdout}");
    assert!(stdout.contains("run"), "stdout: {stdout}");
    // `serve` and `service` are gated behind the `webui-v2-beta` Cargo
    // feature so a default binary build does not link the beta HTTP/auth
    // gateway or the OS-service installer that runs it. The dedicated
    // `serve_*`/`service_*` tests below also `#[cfg]` themselves.
    #[cfg(feature = "webui-v2-beta")]
    assert!(stdout.contains("serve"), "stdout: {stdout}");
    #[cfg(feature = "webui-v2-beta")]
    assert!(stdout.contains("service"), "stdout: {stdout}");
    assert!(stdout.contains("skills"), "stdout: {stdout}");
    // No standalone `tui` subcommand exists (Reborn's interactive surface
    // is `repl`); pin this so a `full`-feature build never grows one
    // without an explicit, reviewed decision.
    assert!(
        !stdout.to_lowercase().contains("tui"),
        "unexpected tui subcommand: {stdout}"
    );
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn service_help_lists_all_verbs() {
    let output = Command::new(reborn_bin())
        .arg("service")
        .arg("--help")
        .output()
        .expect("ironclaw-reborn service --help should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for verb in ["install", "start", "stop", "status", "restart", "uninstall"] {
        assert!(stdout.contains(verb), "missing `{verb}` verb: {stdout}");
    }
}

#[test]
fn extension_search_does_not_seed_reborn_config() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = reborn_command()
        .args(["extension", "search", "--json"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", temp.path().join("home"))
        .output()
        .expect("ironclaw-reborn extension search should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !reborn_home.join("config.toml").exists(),
        "extension search must not seed runtime config"
    );
}

#[test]
fn profile_list_shows_supported_profiles_without_reborn_home() {
    let output = reborn_command()
        .arg("profile")
        .arg("list")
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
    assert!(stdout.contains("local-dev-yolo"), "stdout: {stdout}");
    assert!(stdout.contains("hosted-single-tenant"), "stdout: {stdout}");
    assert!(
        stdout.contains("hosted-single-tenant-volume"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("production"), "stdout: {stdout}");
    assert!(stdout.contains("migration-dry-run"), "stdout: {stdout}");
    assert!(
        stdout.contains("IRONCLAW_REBORN_PROFILE"),
        "stdout: {stdout}"
    );
}

#[test]
fn profile_list_json_is_stable_and_does_not_resolve_reborn_home() {
    let output = reborn_command()
        .arg("profile")
        .arg("list")
        .arg("--json")
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
    assert_eq!(profiles.len(), 6);
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "local-dev" && profile["default"] == true)
    );
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "local-dev-yolo" && profile["default"] == false)
    );
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "hosted-single-tenant"
                && profile["default"] == false)
    );
    assert!(
        profiles
            .iter()
            .any(|profile| profile["name"] == "hosted-single-tenant-volume"
                && profile["default"] == false)
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
fn skills_list_reports_reborn_skill_data() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let v1_home = temp.path().join("v1-home");
    write_reborn_skill(&reborn_home, "catalog-helper", "catalog helper");

    let output = reborn_command()
        .arg("skills")
        .arg("list")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_BASE_DIR", &v1_home)
        .output()
        .expect("ironclaw-reborn skills list should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn skills"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("configured:"), "stdout: {stdout}");
    assert!(
        stdout.contains("source: reborn-local-dev"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("- code-review (system)"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("- catalog-helper (user)"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains("not-wired"), "stdout: {stdout}");
    assert!(!stdout.contains("v1_state"), "stdout: {stdout}");
    assert!(
        !reborn_home
            .join("local-dev/system/skills/code-review/SKILL.md")
            .exists(),
        "skills list should report bundled skills without installing them"
    );
    assert!(
        !v1_home.exists(),
        "skills list must not create or read v1 state"
    );
}

#[test]
fn skills_list_verbose_reports_reborn_skill_details() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    write_verbose_reborn_skill(&reborn_home, "verbose-helper", "verbose helper");

    let output = reborn_command()
        .arg("skills")
        .arg("list")
        .arg("--verbose")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn skills list --verbose should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("profile: local-dev"), "stdout: {stdout}");
    assert!(stdout.contains("reborn_home:"), "stdout: {stdout}");
    assert!(stdout.contains("local_dev_root:"), "stdout: {stdout}");
    assert!(stdout.contains("owner_id: reborn-cli"), "stdout: {stdout}");
    assert!(stdout.contains("version: 1.2.3"), "stdout: {stdout}");
    assert!(
        stdout.contains("keywords: catalog, helper"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("tags: local-dev"), "stdout: {stdout}");
    assert!(
        stdout.contains("requires_skills: companion-helper"),
        "stdout: {stdout}"
    );
}

#[test]
fn skills_list_json_reports_reborn_skill_data() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    write_reborn_skill(&reborn_home, "json-helper", "json helper");

    let output = reborn_command()
        .arg("skills")
        .arg("list")
        .arg("--json")
        .arg("--verbose")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn skills list --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert!(
        json["configured"].as_u64().expect("configured count") > 1,
        "json: {json}"
    );
    assert_eq!(json["source"], "reborn-local-dev");
    assert_skill_source(&json, "code-review", "system");
    assert_skill_source(&json, "json-helper", "user");
    assert_eq!(json["details"]["profile"], "local-dev");
    assert_eq!(json["details"]["owner_id"], "reborn-cli");
    assert!(json.get("limit").is_none(), "json: {json}");
    assert!(json.get("truncated").is_none(), "json: {json}");
    assert!(json.get("status").is_none(), "json: {json}");
    assert!(json.get("v1_state").is_none(), "json: {json}");
}

fn assert_skill_source(json: &serde_json::Value, name: &str, source: &str) {
    let skills = json["skills"].as_array().expect("skills array");
    let skill = skills
        .iter()
        .find(|skill| skill["name"] == name)
        .unwrap_or_else(|| panic!("missing skill {name}: {json}"));
    assert_eq!(skill["source"], source);
}

#[test]
fn skills_list_rejects_unsupported_profiles() {
    for profile in ["production", "migration-dry-run"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = reborn_command()
            .arg("skills")
            .arg("list")
            .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
            .env("IRONCLAW_REBORN_PROFILE", profile)
            .output()
            .expect("ironclaw-reborn skills list should run");

        assert!(
            !output.status.success(),
            "skills list should reject profile={profile}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("ironclaw-reborn skills currently supports profile=local-dev"),
            "stderr: {stderr}"
        );
        assert!(
            stderr.contains(&format!("profile={profile}")),
            "stderr: {stderr}"
        );
    }
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

#[cfg(feature = "root-llm-provider")]
#[test]
fn models_list_reports_reborn_provider_catalog_without_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = reborn_command()
        .arg("models")
        .arg("list")
        .env("HOME", temp.path())
        .output()
        .expect("ironclaw-reborn models list should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn LLM providers"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("providers_file:"), "stdout: {stdout}");
    assert!(
        stdout.contains("active: not-configured"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("openai"), "stdout: {stdout}");
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
}

#[cfg(feature = "root-llm-provider")]
#[test]
fn models_status_json_reports_routes_not_configured_without_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = reborn_command()
        .arg("models")
        .arg("status")
        .arg("--json")
        .env("HOME", temp.path())
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
    assert_eq!(json["default"], serde_json::Value::Null);
    assert_eq!(json["v1_state"], "not-used");
}

#[cfg(feature = "root-llm-provider")]
#[test]
fn models_status_reads_reborn_default_llm_slot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[llm.default]
provider_id = "openai"
model = "gpt-5-mini"
api_key_env = "OPENAI_API_KEY"
"#,
    )
    .expect("write config");

    let output = reborn_command()
        .arg("models")
        .arg("status")
        .arg("--json")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models status --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(json["routes"], "configured");
    assert_eq!(json["default"]["provider_id"], "openai");
    assert_eq!(json["default"]["provider_known"], true);
    assert_eq!(json["default"]["model"], "gpt-5-mini");
    assert_eq!(json["default"]["api_key_env"], "OPENAI_API_KEY");
    assert_eq!(json["v1_state"], "not-used");
}

#[cfg(feature = "root-llm-provider")]
#[test]
fn models_set_provider_writes_reborn_config_without_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let output = reborn_command()
        .arg("models")
        .arg("set-provider")
        .arg("openai")
        .arg("--model")
        .arg("gpt-5-mini")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set-provider should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Provider set to `openai`"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");

    let config = std::fs::read_to_string(reborn_home.join("config.toml")).expect("read config");
    assert!(config.contains("[llm.default]"), "config: {config}");
    assert!(
        config.contains("provider_id = \"openai\""),
        "config: {config}"
    );
    assert!(
        config.contains("model = \"gpt-5-mini\""),
        "config: {config}"
    );
    assert!(
        config.contains("api_key_env = \"OPENAI_API_KEY\""),
        "config: {config}"
    );
    assert!(
        !temp.path().join(".ironclaw").join(".env").exists(),
        "Reborn models set-provider must not write v1 bootstrap .env"
    );
}

#[cfg(feature = "root-llm-provider")]
#[test]
fn models_set_updates_reborn_default_model() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[llm.default]
provider_id = "openai"
model = "gpt-5-mini"
api_key_env = "OPENAI_API_KEY"
"#,
    )
    .expect("write config");

    let output = reborn_command()
        .arg("models")
        .arg("set")
        .arg("gpt-5.3-codex")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config = std::fs::read_to_string(reborn_home.join("config.toml")).expect("read config");
    assert!(
        config.contains("provider_id = \"openai\""),
        "config: {config}"
    );
    assert!(
        config.contains("model = \"gpt-5.3-codex\""),
        "config: {config}"
    );
}

#[cfg(feature = "root-llm-provider")]
#[test]
fn models_set_without_provider_fails_without_panicking() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let output = reborn_command()
        .arg("models")
        .arg("set")
        .arg("gpt-5.3-codex")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set should run");

    assert!(!output.status.success(), "models set should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no default Reborn provider is configured"),
        "stderr: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
}

#[cfg(not(feature = "root-llm-provider"))]
#[test]
fn models_list_no_default_features_does_not_resolve_reborn_home() {
    let output = reborn_command()
        .arg("models")
        .arg("list")
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
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
}

#[cfg(not(feature = "root-llm-provider"))]
#[test]
fn models_status_no_default_features_does_not_resolve_reborn_home() {
    let output = reborn_command()
        .arg("models")
        .arg("status")
        .arg("--json")
        .output()
        .expect("ironclaw-reborn models status should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(json["routes"], "not-configured");
    assert_eq!(json["v1_state"], "not-used");
}

#[cfg(not(feature = "root-llm-provider"))]
#[test]
fn models_write_commands_report_root_llm_provider_required_without_default_features() {
    for args in [
        &["models", "set", "gpt-5.3-codex"][..],
        &["models", "set-provider", "openai"][..],
    ] {
        let output = reborn_command()
            .args(args)
            .output()
            .expect("ironclaw-reborn models write command should run");

        assert!(!output.status.success(), "command should fail: {args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("requires the root-llm-provider feature"),
            "stderr: {stderr}"
        );
        assert!(stderr.contains("v1_state: not-used"), "stderr: {stderr}");
        assert!(
            !stderr.contains("HOME or USERPROFILE"),
            "must not resolve Reborn home before feature error: {stderr}"
        );
    }
}

fn assert_empty_not_wired_surface(
    args: &[&str],
    title: &str,
    collection_key: &str,
    count_key: &str,
) {
    let output = reborn_command()
        .args(args)
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
    let output = reborn_command()
        .args(json_args)
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

fn write_reborn_skill(reborn_home: &std::path::Path, name: &str, description: &str) {
    let skill_dir = reborn_cli_skill_root(reborn_home).join(name);
    std::fs::create_dir_all(&skill_dir).expect("skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\nUse {name}.\n"),
    )
    .expect("skill file");
}

fn write_verbose_reborn_skill(reborn_home: &std::path::Path, name: &str, description: &str) {
    let skill_dir = reborn_cli_skill_root(reborn_home).join(name);
    std::fs::create_dir_all(&skill_dir).expect("skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!(
            r#"---
name: {name}
version: "1.2.3"
description: {description}
activation:
  keywords: ["catalog", "helper"]
  tags: ["local-dev"]
requires:
  skills: ["companion-helper"]
---
Use {name}.
"#
        ),
    )
    .expect("skill file");
}

fn reborn_cli_skill_root(reborn_home: &std::path::Path) -> std::path::PathBuf {
    reborn_home.join("local-dev/tenants/default/users/reborn-cli/skills")
}

fn assert_verbose_detail(args: &[&str], expected_detail: &str) {
    let output = reborn_command()
        .args(args)
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
    let output = reborn_command()
        .args(args)
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
    let output = reborn_command()
        .arg("completion")
        .arg("--shell")
        .arg("zsh")
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
    let output = reborn_command()
        .arg("completion")
        .arg("--shell")
        .arg("bash")
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

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_help_mentions_host_and_port() {
    let output = reborn_command()
        .arg("serve")
        .arg("--help")
        .output()
        .expect("ironclaw-reborn serve --help should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--host"), "stdout: {stdout}");
    assert!(stdout.contains("--port"), "stdout: {stdout}");
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_fails_closed_when_env_bearer_token_var_is_unset() {
    // The standalone CLI's env-bearer authenticator reads the token
    // value out of the env var named by `[webui].env_token_var`
    // (defaulting to IRONCLAW_REBORN_WEBUI_TOKEN). When that var is
    // absent the CLI must exit non-zero before binding any listener —
    // we never want a half-configured serve loop running with auth
    // disabled.
    let temp = tempfile::tempdir().expect("tempdir");

    let output = Command::new(reborn_bin())
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg("0")
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .env_remove("IRONCLAW_REBORN_WEBUI_TOKEN")
        .env_remove("IRONCLAW_REBORN_WEBUI_USER_ID")
        .output()
        .expect("ironclaw-reborn serve should run");

    assert!(
        !output.status.success(),
        "serve must fail closed when the bearer token env var is unset"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_WEBUI_TOKEN must be set"),
        "stderr should explain which env var is missing: {stderr}"
    );
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_boots_without_user_id_env_var() {
    // Regression for the Railway/service-install crash-loop root cause: a
    // launchd/systemd/Railway unit whose environment carries only
    // HOME/PROFILE (see serve_invocation.rs) never sets
    // IRONCLAW_REBORN_WEBUI_USER_ID. `serve` previously hard-failed before
    // binding any listener; it must now fall back to the config file's
    // `[identity].default_owner` (or the hard-coded "reborn-cli" default
    // when `[identity]` is absent, as here, since no config.toml is
    // seeded) instead of exiting.
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        // >=32 bytes: must clear the token's own entropy floor (enforced by
        // `webui_token::resolve_webui_token` as soon as the token is
        // resolved, before the user-id var is read) so this test isolates
        // the user-id-var-absent fallback it's meant to exercise.
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "reborn-smoke-test-token-0123456789abcdef",
        )
        .env_remove("IRONCLAW_REBORN_WEBUI_USER_ID")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn a_real_env_var_beats_the_config_default_end_to_end() {
    // Railway non-regression spine: the well-trodden Railway/service-install
    // path (operator sets IRONCLAW_REBORN_WEBUI_USER_ID explicitly, no
    // `[identity].default_owner` in config.toml) must keep booting after
    // user-id resolution moved from a bare `env::var(...)?` into the shared
    // `resolve_webui_user_id_raw` fallback helper. `[identity]` is
    // deliberately left unset here: `resolve_webui_runtime_owner` rejects
    // any *configured* `default_owner` that disagrees with the resolved
    // WebUI user by design (see `webui_runtime_owner_rejects_divergent_config_owner`),
    // so a config default that differs from the env value is a distinct,
    // already-covered operator-misconfiguration case, not this one. Exact
    // env-over-config-default precedence is pinned at the unit level by
    // `webui_user_id_raw_prefers_a_set_nonempty_env_var`; this test proves
    // the env-set path still reaches a bound listener end-to-end.
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "reborn-smoke-test-token-0123456789abcdef",
        )
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "env-user")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_with_env_auth_seeds_reborn_config_before_binding() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            // >=32 bytes: serve now enforces the session-signing entropy
            // floor unconditionally (it signs admin-minted session tokens
            // even without SSO).
            "reborn-smoke-test-token-0123456789abcdef",
        )
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    let stderr = child.stderr.take().expect("stderr should be piped");
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines() {
            if stderr_tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut stderr_text = String::new();
    loop {
        if let Some(status) = child.try_wait().expect("serve child status") {
            panic!("serve exited before binding with {status}; stderr: {stderr_text}");
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("serve did not reach listener banner; stderr: {stderr_text}");
        }
        match stderr_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Ok(line)) => {
                stderr_text.push_str(&line);
                stderr_text.push('\n');
                if stderr_text.contains("ironclaw-reborn: WebChat v2 listener") {
                    break;
                }
            }
            Ok(Err(error)) => panic!("failed to read serve stderr: {error}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("serve stderr closed before banner; stderr: {stderr_text}");
            }
        }
    }

    let providers_status = match http_status_line(
        port,
        concat!(
            "GET /auth/providers HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n",
        ),
        "providers route probe",
    ) {
        Ok(status_line) => status_line,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{error}");
        }
    };
    let logout_status = match http_status_line(
        port,
        concat!(
            "POST /auth/logout HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Authorization: Bearer test-token\r\n",
            "Content-Length: 0\r\n",
            "Connection: close\r\n",
            "\r\n",
        ),
        "logout route probe",
    ) {
        Ok(status_line) => status_line,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{error}");
        }
    };

    let _ = child.kill();
    let _ = child.wait();
    assert!(
        providers_status.contains(" 200 "),
        "no-SSO serve must still expose empty provider discovery, got status line: {providers_status}"
    );
    assert!(
        logout_status.contains(" 404 "),
        "no-SSO env-bearer serve must not mount logout, got status line: {logout_status}"
    );
    let config = std::fs::read_to_string(reborn_home.join("config.toml"))
        .expect("successful serve startup should seed config");
    assert!(
        config.contains("api_version = \"ironclaw.runtime/v1\""),
        "seeded config should stamp api_version: {config}"
    );
    assert!(
        config.contains("profile = \"local-dev\""),
        "seeded config should preserve the safe default profile: {config}"
    );
    assert!(
        !config.contains("[llm.default]"),
        "serve seed must preserve no-LLM behavior: {config}"
    );
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_resolves_bearer_token_from_reborn_home_webui_token_file() {
    // Regression for the service-install crash loop: a launchd/systemd unit
    // whose environment carries only HOME/PROFILE (see serve_invocation.rs)
    // never sets IRONCLAW_REBORN_WEBUI_TOKEN, so `serve` must also accept
    // the `onboard`-provisioned `<reborn_home>/webui-token` fallback file.
    // Mirrors `serve_with_env_auth_seeds_reborn_config_before_binding` but
    // omits the env var and seeds the file instead.
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&reborn_home).expect("reborn home dir");
    std::fs::write(
        reborn_home.join("webui-token"),
        // >=32 bytes: same entropy floor as the env-var path.
        "reborn-smoke-test-token-0123456789abcdef",
    )
    .expect("seed webui-token file");
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_WEBUI_TOKEN")
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    let stderr = child.stderr.take().expect("stderr should be piped");
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines() {
            if stderr_tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut stderr_text = String::new();
    loop {
        if let Some(status) = child.try_wait().expect("serve child status") {
            panic!("serve exited before binding with {status}; stderr: {stderr_text}");
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("serve did not reach listener banner; stderr: {stderr_text}");
        }
        match stderr_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Ok(line)) => {
                stderr_text.push_str(&line);
                stderr_text.push('\n');
                if stderr_text.contains("ironclaw-reborn: WebChat v2 listener") {
                    break;
                }
            }
            Ok(Err(error)) => panic!("failed to read serve stderr: {error}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("serve stderr closed before banner; stderr: {stderr_text}");
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(feature = "webui-v2-beta", feature = "slack-v2-host-beta"))]
#[test]
fn serve_env_slack_enabled_mounts_slack_events_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            // >=32 bytes: serve now enforces the session-signing entropy
            // floor unconditionally (it signs admin-minted session tokens
            // even without SSO).
            "reborn-smoke-test-token-0123456789abcdef",
        )
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .env("IRONCLAW_REBORN_SLACK_ENABLED", "true")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    let stderr = child.stderr.take().expect("stderr should be piped");
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines() {
            if stderr_tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut stderr_text = String::new();
    loop {
        if let Some(status) = child.try_wait().expect("serve child status") {
            panic!("serve exited before binding with {status}; stderr: {stderr_text}");
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("serve did not reach listener banner; stderr: {stderr_text}");
        }
        match stderr_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Ok(line)) => {
                stderr_text.push_str(&line);
                stderr_text.push('\n');
                if stderr_text.contains("ironclaw-reborn: WebChat v2 listener") {
                    break;
                }
            }
            Ok(Err(error)) => panic!("failed to read serve stderr: {error}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("serve stderr closed before banner; stderr: {stderr_text}");
            }
        }
    }

    let status_line = match post_slack_events_status_line(port) {
        Ok(status_line) => status_line,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{error}");
        }
    };

    let _ = child.kill();
    let _ = child.wait();
    assert!(
        !status_line.contains(" 404 "),
        "env-enabled Slack route should be mounted, got status line: {status_line}"
    );
}

#[cfg(feature = "webui-v2-beta")]
fn unused_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral local port")
        .local_addr()
        .expect("ephemeral local addr")
        .port()
}

#[cfg(feature = "webui-v2-beta")]
fn http_status_line(port: u16, request: &str, label: &str) -> Result<String, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(error) => return Err(format!("connect to serve listener failed: {error}")),
        }
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|error| format!("set {label} read timeout failed: {error}"))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("write {label} failed: {error}"))?;
    let mut reader = std::io::BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|error| format!("read {label} status line failed: {error}"))?;
    Ok(status_line)
}

#[cfg(all(feature = "webui-v2-beta", feature = "slack-v2-host-beta"))]
fn post_slack_events_status_line(port: u16) -> Result<String, String> {
    http_status_line(
        port,
        concat!(
            "POST /webhooks/slack/events HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Content-Type: application/json\r\n",
            "Content-Length: 2\r\n",
            "Connection: close\r\n",
            "\r\n",
            "{}"
        ),
        "Slack route probe",
    )
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_rejects_malformed_host_before_webui_handoff() {
    let temp = tempfile::tempdir().expect("tempdir");

    let output = Command::new(reborn_bin())
        .arg("serve")
        .arg("--host")
        .arg("localhost:3000")
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .output()
        .expect("ironclaw-reborn serve should run");

    assert!(
        !output.status.success(),
        "serve should reject malformed host"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid value"), "stderr: {stderr}");
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_rejects_invalid_webui_security_config_before_binding() {
    let cases = [
        (
            r#"
[webui]
canonical_host = "https://app.example.com"
"#,
            "[webui].canonical_host `https://app.example.com` must be `host` or `host:port`",
        ),
        (
            r#"
[webui]
allowed_origins = ["https://app.example.com", "bad\norigin"]
"#,
            "[webui].allowed_origins parse failure",
        ),
        (
            r#"
[webui]
max_body_bytes_fallback = 0
"#,
            "[webui].max_body_bytes_fallback must be > 0",
        ),
    ];

    for (config, expected) in cases {
        let temp = tempfile::tempdir().expect("tempdir");
        let reborn_home = temp.path().join("reborn-home");
        std::fs::create_dir_all(&reborn_home).expect("reborn home");
        std::fs::write(reborn_home.join("config.toml"), config).expect("write config");

        let output = isolated_no_llm_command(temp.path(), &reborn_home)
            .args(["serve", "--host", "127.0.0.1", "--port", "0"])
            .env(
                "IRONCLAW_REBORN_WEBUI_TOKEN",
                // >=32 bytes: serve now enforces the session-signing entropy
                // floor unconditionally (it signs admin-minted session tokens
                // even without SSO).
                "reborn-smoke-test-token-0123456789abcdef",
            )
            .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
            .output()
            .expect("ironclaw-reborn serve should not crash");

        assert!(
            !output.status.success(),
            "invalid WebUI security config must fail closed before binding"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected),
            "stderr should contain {expected:?}; got: {stderr}"
        );
        assert!(
            !stderr.contains("ironclaw-reborn: WebChat v2 listener"),
            "serve must not bind after invalid WebUI security config; got: {stderr}"
        );
    }
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_fails_closed_when_sso_provider_has_no_allowed_domain_allowlist() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = isolated_no_llm_command(temp.path(), &reborn_home)
        .args(["serve", "--host", "127.0.0.1", "--port", "0"])
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "0123456789abcdef0123456789abcdef",
        )
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID", "client-id")
        .env(
            "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET",
            "client-secret",
        )
        .output()
        .expect("ironclaw-reborn serve should not crash");

    assert!(
        !output.status.success(),
        "serve must fail closed when SSO is configured without an admission allowlist"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("WebChat v2 SSO providers are configured")
            && stderr.contains("IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS")
            && stderr.contains("open registration"),
        "stderr should explain the missing SSO admission allowlist; got: {stderr}"
    );
    assert!(
        !stderr.contains("ironclaw-reborn: WebChat v2 listener"),
        "serve must not bind after SSO admission misconfiguration; got: {stderr}"
    );
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_fails_closed_when_session_token_lacks_entropy_without_sso() {
    // Regression for the offline HMAC-oracle gap: serve always wires the admin
    // API token minter, which signs user-visible session tokens from the env
    // bearer secret. A weak secret is therefore an offline forgery target even
    // when no SSO provider is configured, so the >=32-byte entropy floor must
    // fire unconditionally — not only when SSO startup is present.
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = isolated_no_llm_command(temp.path(), &reborn_home)
        .args(["serve", "--host", "127.0.0.1", "--port", "0"])
        // 16 bytes: below the floor, and NO SSO provider env is set.
        .env("IRONCLAW_REBORN_WEBUI_TOKEN", "short-weak-token")
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .output()
        .expect("ironclaw-reborn serve should not crash");

    assert!(
        !output.status.success(),
        "serve must fail closed on a low-entropy session-signing secret even without SSO"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("session-signing key") && stderr.contains("at least 32 bytes"),
        "stderr should explain the session-signing entropy floor; got: {stderr}"
    );
    assert!(
        !stderr.contains("ironclaw-reborn: WebChat v2 listener"),
        "serve must not bind with a low-entropy session-signing secret; got: {stderr}"
    );
}

/// Send `request` and read the full response: status line, headers, and
/// (best-effort, non-chunked) body. Used by the CLI-token-login tests below,
/// which need `Location`/JSON body content that [`http_status_line`] doesn't
/// capture.
#[cfg(feature = "webui-v2-beta")]
fn http_response(port: u16, request: &str, label: &str) -> Result<HttpResponse, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(error) => return Err(format!("connect to serve listener failed: {error}")),
        }
    };
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|error| format!("set {label} read timeout failed: {error}"))?;
    let mut stream = stream;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("write {label} failed: {error}"))?;
    let mut reader = std::io::BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|error| format!("read {label} status line failed: {error}"))?;
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| format!("read {label} header line failed: {error}"))?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    let mut body = String::new();
    std::io::Read::read_to_string(&mut reader, &mut body)
        .map_err(|error| format!("read {label} body failed: {error}"))?;
    Ok(HttpResponse {
        status_line: status_line.trim_end_matches(['\r', '\n']).to_string(),
        headers,
        body,
    })
}

#[cfg(feature = "webui-v2-beta")]
#[derive(Debug)]
struct HttpResponse {
    status_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

#[cfg(feature = "webui-v2-beta")]
impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name == name)
            .map(|(_, value)| value.as_str())
    }
}

/// RED (B4 step 1 / exchange-handler collision), narrowed by the
/// file-vs-env token-source fix: with no SSO provider configured AND the
/// resolved webui token sourced from the token FILE (not an env var —
/// `serve` only mounts the CLI-login route for a file-sourced token; see
/// `commands::serve::execute`'s `cli_login_mount` condition), `serve` must
/// mount the CLI-printed `/login?token=` route
/// (`ironclaw_reborn_webui_ingress::build_cli_token_login`) alongside its own
/// `POST /auth/session/exchange`. A valid token redirects into the ticket
/// hand-off; the ticket then resolves to a real session bearer through the
/// exchange route. An invalid token gets a flat 401 with no ticket minted.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_mounts_cli_login_route_without_sso() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let port = unused_local_port();
    let webui_token = "reborn-smoke-test-token-0123456789abcdef";
    // File-sourced token, not `IRONCLAW_REBORN_WEBUI_TOKEN` — this is the
    // one source `cli_login_mount` still mounts the route for.
    std::fs::create_dir_all(&reborn_home).expect("reborn home dir");
    std::fs::write(reborn_home.join("webui-token"), webui_token).expect("seed webui-token file");

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    let wrong_token_status = http_response(
        port,
        "GET /login?token=wrong HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        "login wrong-token probe",
    );
    let good_login = http_response(
        port,
        &format!(
            "GET /login?token={webui_token} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        ),
        "login probe",
    );

    let (wrong_token_status, good_login) = match (wrong_token_status, good_login) {
        (Ok(a), Ok(b)) => (a, b),
        (result_a, result_b) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("login probe failed: {result_a:?} / {result_b:?}");
        }
    };

    assert!(
        wrong_token_status.status_line.contains(" 401 "),
        "wrong token must 401, got: {}",
        wrong_token_status.status_line
    );
    assert!(
        good_login.status_line.contains(" 302 ") || good_login.status_line.contains(" 303 "),
        "valid token must redirect into the ticket hand-off, got: {}",
        good_login.status_line
    );
    let location = good_login
        .header("location")
        .expect("redirect must carry a Location header");
    let ticket = location
        .split("login_ticket=")
        .nth(1)
        .expect("redirect Location must carry a login_ticket query param");

    let exchange_body = format!(r#"{{"ticket":"{ticket}"}}"#);
    let exchange_request = format!(
        "POST /auth/session/exchange HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{exchange_body}",
        exchange_body.len()
    );
    let exchange = http_response(port, &exchange_request, "session exchange probe");

    let _ = child.kill();
    let _ = child.wait();

    let exchange = exchange.expect("exchange probe must complete");
    assert!(
        exchange.status_line.contains(" 200 "),
        "ticket must exchange for a real bearer exactly once, got: {}; body: {}",
        exchange.status_line,
        exchange.body
    );
    assert!(
        exchange.body.contains("\"token\""),
        "exchange response must carry the minted bearer: {}",
        exchange.body
    );
}

/// Security fix: when the webui bearer token comes from the env var
/// (`IRONCLAW_REBORN_WEBUI_TOKEN`, the Railway deployment shape — profile
/// local-dev, no SSO, the env var set), `serve` must NOT mount the
/// CLI-token `/login?token=` route. That route puts the bearer in a public
/// route's URL query string, where an edge/proxy would capture it in access
/// logs; a file-sourced token has no such env-carried master credential to
/// protect. Port-contention flaky like its file-token sibling above — same
/// spawn-and-poll isolation pattern.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_does_not_mount_cli_login_route_when_token_is_env_sourced() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "reborn-smoke-test-env-token-0123456789abcdef",
        )
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    let login_status = http_status_line(
        port,
        "GET /login?token=irrelevant HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        "cli login probe with env-sourced token",
    );

    let _ = child.kill();
    let _ = child.wait();

    let login_status = login_status.expect("login probe must complete");
    assert!(
        login_status.contains(" 404 "),
        "the CLI-only /login route must not mount for an env-sourced token, got: {login_status}"
    );
}

/// RED (B4 step 1 / exchange-handler collision): when an SSO provider IS
/// configured, `serve` must NOT also mount the CLI-token-login route's own
/// `/auth/session/exchange` — that would register the path twice. The SSO
/// login surface's own exchange route stays the only one, proven here by the
/// CLI-only `/login?token=` route being absent (404) while the SSO surface's
/// `/auth/providers` stays up.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_with_sso_does_not_double_mount_session_exchange() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "reborn-smoke-test-token-0123456789abcdef",
        )
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID", "client-id")
        .env(
            "IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET",
            "client-secret",
        )
        .env("IRONCLAW_REBORN_WEBUI_ALLOWED_EMAIL_DOMAINS", "example.com")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    let login_status = http_status_line(
        port,
        "GET /login?token=irrelevant HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        "cli login probe under SSO",
    );
    let providers_status = http_status_line(
        port,
        concat!(
            "GET /auth/providers HTTP/1.1\r\n",
            "Host: 127.0.0.1\r\n",
            "Connection: close\r\n",
            "\r\n",
        ),
        "providers route probe",
    );

    let _ = child.kill();
    let _ = child.wait();

    let login_status = login_status.expect("cli login probe must complete");
    let providers_status = providers_status.expect("providers probe must complete");
    assert!(
        login_status.contains(" 404 "),
        "the CLI-only /login route must not mount when SSO is configured \
         (its own /auth/session/exchange would collide with the SSO surface's), got: {login_status}"
    );
    assert!(
        providers_status.contains(" 200 "),
        "the SSO surface's own routes (including its /auth/session/exchange) \
         must still be the sole mount, got: {providers_status}"
    );
}

/// Strip ANSI SGR escape sequences (`\x1b[...m`) from `text`. `init_tracing`'s
/// stderr `fmt::layer()` colorizes its output unconditionally — it does not
/// gate on the writer being a real terminal — so a piped `Child::stderr`
/// still carries color codes interleaved between field names and values
/// (e.g. `provider_id` and `=openai` are split by a reset/dim escape pair).
/// Assertions on structured-log field text must strip these first or a
/// plain `contains("provider_id=openai")` silently never matches.
#[cfg(feature = "webui-v2-beta")]
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.clone().next() == Some('[') {
            chars.next(); // consume '['
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Spawn-and-poll helper shared by the CLI-token-login tests above: block
/// until `child`'s stderr carries the ready banner, matching the polling
/// shape every other `serve` smoke test in this file hand-rolls inline.
/// Returns everything captured on stderr up to and including the banner
/// line, so callers that need to assert on pre-banner diagnostics (e.g. the
/// resolved-LLM `debug!` trace emitted during boot, before
/// `print_serve_banner` runs — see `runtime::build_runtime_input_with_options`)
/// don't need their own stderr-draining thread.
#[cfg(feature = "webui-v2-beta")]
fn wait_for_serve_banner(child: &mut std::process::Child) -> String {
    let stderr = child.stderr.take().expect("stderr should be piped");
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines() {
            if stderr_tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut stderr_text = String::new();
    loop {
        if let Some(status) = child.try_wait().expect("serve child status") {
            panic!("serve exited before binding with {status}; stderr: {stderr_text}");
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("serve did not reach listener banner; stderr: {stderr_text}");
        }
        match stderr_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Ok(line)) => {
                stderr_text.push_str(&line);
                stderr_text.push('\n');
                if stderr_text.contains("ironclaw-reborn: WebChat v2 listener") {
                    break;
                }
            }
            Ok(Err(error)) => panic!("failed to read serve stderr: {error}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("serve stderr closed before banner; stderr: {stderr_text}");
            }
        }
    }
    stderr_text
}

// Note: port `0` is intentionally accepted now — it lets the kernel
// pick a free port, which is the path the caller-level serve test
// uses to avoid hard-coding a port. The earlier zero-port rejection
// belonged to the stub serve loop that never actually bound.
//
// Banner formatting (IPv6 / IPv4 / config readout) is exercised by
// the caller-level test in
// `ironclaw_webui::tests` rather than from the binary
// smoke test, because the banner is printed AFTER env-token resolution
// + runtime build, both of which require a configured environment.

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
    assert!(stdout.contains("local-dev"), "stdout: {stdout}");
    assert!(stdout.contains("text_only_driver"), "stdout: {stdout}");
    assert!(
        !stdout.contains("v1_state"),
        "doctor output should not include v1_state"
    );
    assert!(
        !reborn_home.exists(),
        "doctor should not create state directories"
    );
}

#[test]
fn repl_help_mentions_composed_runtime() {
    let output = reborn_command()
        .arg("repl")
        .arg("--help")
        .output()
        .expect("ironclaw-reborn repl --help should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("composed Reborn CLI REPL"),
        "stdout: {stdout}"
    );
}

#[test]
fn repl_exit_command_seeds_reborn_config() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home_dir = temp.path().join("home");
    let v1_base_dir = temp.path().join("v1-state");

    let mut child = reborn_command()
        .arg("repl")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", &home_dir)
        .env("IRONCLAW_BASE_DIR", &v1_base_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn repl should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"/exit\n")
        .expect("exit command should be written");
    let output = child
        .wait_with_output()
        .expect("ironclaw-reborn repl should finish");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout should stay reply-only: {stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ironclaw-reborn: runtime started"),
        "stderr: {stderr}"
    );
    assert!(
        !home_dir.join(".ironclaw").exists(),
        "repl should not create default v1 state directories"
    );
    assert!(
        !v1_base_dir.exists(),
        "repl should not create explicit v1 base directories"
    );
    let config_path = reborn_home.join("config.toml");
    let config = std::fs::read_to_string(&config_path).unwrap_or_else(|err| {
        panic!(
            "first stateful repl start should seed {}: {err}",
            config_path.display()
        )
    });
    assert!(
        config.contains("api_version = \"ironclaw.runtime/v1\""),
        "seeded config should stamp api_version: {config}"
    );
    assert!(
        config.contains("profile = \"local-dev\""),
        "seeded config should record default profile: {config}"
    );
    assert!(
        !config.contains("[llm.default]"),
        "first-run seed must preserve no-LLM behavior: {config}"
    );
}

#[test]
fn repl_resolves_codex_auth_env_without_openai_api_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home_dir = temp.path().join("home");
    let codex_auth_path = temp.path().join("codex-auth.json");
    std::fs::write(
        &codex_auth_path,
        r#"{
  "auth_mode": "chatgpt",
  "tokens": {
    "access_token": "test-access-token",
    "refresh_token": "test-refresh-token"
  }
}
"#,
    )
    .expect("write codex auth fixture");

    let mut child = reborn_command()
        .arg("repl")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", &home_dir)
        .env("LLM_BACKEND", "openai_codex")
        .env("LLM_USE_CODEX_AUTH", "true")
        .env("CODEX_AUTH_PATH", &codex_auth_path)
        .env("OPENAI_CODEX_MODEL", "gpt-test-codex")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn repl should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"/exit\n")
        .expect("exit command should be written");
    let output = child
        .wait_with_output()
        .expect("ironclaw-reborn repl should finish");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ironclaw-reborn: runtime started"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("no LLM selection configured"),
        "Codex auth should prevent stub-gateway warning: {stderr}"
    );
}

#[test]
fn repl_resolves_codex_api_key_auth_env_without_openai_api_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home_dir = temp.path().join("home");
    let codex_auth_path = temp.path().join("codex-auth.json");
    std::fs::write(
        &codex_auth_path,
        r#"{
  "auth_mode": "apiKey",
  "OPENAI_API_KEY": "sk-test-codex-api-key"
}
"#,
    )
    .expect("write codex auth fixture");

    let mut child = reborn_command()
        .arg("repl")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", &home_dir)
        .env("LLM_BACKEND", "openai_codex")
        .env("LLM_USE_CODEX_AUTH", "true")
        .env("CODEX_AUTH_PATH", &codex_auth_path)
        .env("OPENAI_CODEX_MODEL", "gpt-test-codex")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn repl should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"/exit\n")
        .expect("exit command should be written");
    let output = child
        .wait_with_output()
        .expect("ironclaw-reborn repl should finish");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ironclaw-reborn: runtime started"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("no LLM selection configured"),
        "Codex API-key auth should prevent stub-gateway warning: {stderr}"
    );
}

// Provider/auth validation lives behind `root-llm-provider` (a default
// feature); the `libsql-only` build drops it and boots a stub, so this test
// only applies when that feature is compiled in.
#[cfg(feature = "root-llm-provider")]
#[test]
fn run_rejects_codex_backend_when_auth_file_is_missing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let missing_codex_auth_path = temp.path().join("missing-codex-auth.json");

    let output = reborn_command()
        .args(["run", "-m", "ping"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("LLM_BACKEND", "openai_codex")
        .env("CODEX_AUTH_PATH", &missing_codex_auth_path)
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(
        !output.status.success(),
        "missing Codex auth should fail; stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Authentication failed for provider 'openai_codex'"),
        "stderr should report Codex auth failure; got: {stderr}"
    );
    assert!(
        !stderr.contains(&missing_codex_auth_path.display().to_string()),
        "stderr should not leak the Codex auth path: {stderr}"
    );
}

#[test]
fn repl_help_command_prints_repl_commands_and_exits_on_exit() {
    let temp = tempfile::tempdir().expect("tempdir");

    let mut child = reborn_command()
        .arg("repl")
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env("HOME", temp.path().join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn repl should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"/help\n/quit\n")
        .expect("repl commands should be written");
    let output = child
        .wait_with_output()
        .expect("ironclaw-reborn repl should finish");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Reborn REPL commands:"), "stderr: {stderr}");
    assert!(stderr.contains("/exit"), "stderr: {stderr}");
    assert!(stderr.contains("/quit"), "stderr: {stderr}");
}

#[test]
fn run_help_command_prints_repl_commands_and_exits_on_quit() {
    let temp = tempfile::tempdir().expect("tempdir");

    let mut child = reborn_command()
        .arg("run")
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
        .write_all(b"/help\n/quit\n")
        .expect("run repl commands should be written");
    let output = child
        .wait_with_output()
        .expect("ironclaw-reborn run should finish");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout should stay reply-only: {stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Reborn REPL commands:"), "stderr: {stderr}");
    assert!(stderr.contains("/exit"), "stderr: {stderr}");
    assert!(stderr.contains("/quit"), "stderr: {stderr}");
}

#[test]
fn repl_piped_message_exits_nonzero_when_runtime_does_not_produce_reply() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let mut child = reborn_command()
        .arg("repl")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", temp.path().join("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn repl should start");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(b"hello\n")
        .expect("prompt should be written");
    let output = child
        .wait_with_output()
        .expect("ironclaw-reborn repl should finish");

    assert!(
        !output.status.success(),
        "repl should fail when the runtime cannot produce assistant text"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout should stay reply-only: {stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("reborn run did not produce an assistant reply"),
        "stderr: {stderr}"
    );
    let config_path = reborn_home.join("config.toml");
    let config = std::fs::read_to_string(&config_path).unwrap_or_else(|err| {
        panic!(
            "first real repl input should seed {}: {err}",
            config_path.display()
        )
    });
    assert!(
        config.contains("api_version = \"ironclaw.runtime/v1\""),
        "seeded config should stamp api_version: {config}"
    );
    assert!(
        config.contains("profile = \"local-dev\""),
        "seeded config should record default profile: {config}"
    );
    assert!(
        !config.contains("[llm.default]"),
        "first-run seed must preserve no-LLM behavior: {config}"
    );
}

#[test]
fn run_message_exits_nonzero_when_runtime_does_not_produce_reply() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = reborn_command()
        .arg("run")
        .arg("--message")
        .arg("hello")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
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

    let config_path = reborn_home.join("config.toml");
    let config = std::fs::read_to_string(&config_path).unwrap_or_else(|err| {
        panic!(
            "first real run should seed {}: {err}",
            config_path.display()
        )
    });
    assert!(
        config.contains("api_version = \"ironclaw.runtime/v1\""),
        "seeded config should stamp api_version: {config}"
    );
    assert!(
        config.contains("profile = \"local-dev\""),
        "seeded config should record default profile: {config}"
    );
    assert!(
        !config.contains("[llm.default]"),
        "first-run seed must preserve no-LLM behavior: {config}"
    );
}

#[test]
fn run_piped_stdin_exits_nonzero_when_runtime_does_not_produce_reply() {
    let temp = tempfile::tempdir().expect("tempdir");

    let mut child = reborn_command()
        .arg("run")
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
    assert!(stdout.contains("(default)"), "stdout: {stdout}");
    assert!(stdout.contains("local-dev"), "stdout: {stdout}");
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
    assert!(
        stdout
            .lines()
            .any(|line| line.contains("profile") && line.contains("production")),
        "expected a line containing both 'profile' and 'production', stdout: {stdout}"
    );
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
    let output = reborn_command()
        .arg("doctor")
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
    let output = reborn_command()
        .arg("doctor")
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
    let output = reborn_command()
        .arg("doctor")
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

#[test]
fn doctor_json_reports_checks_and_summary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .args(["doctor", "--json"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn doctor --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");

    let checks = json["checks"].as_array().expect("checks is array");
    assert!(!checks.is_empty(), "checks should not be empty");
    for check in checks {
        assert!(check.get("name").is_some(), "check must have name");
        assert!(check.get("category").is_some(), "check must have category");
        assert!(check.get("outcome").is_some(), "check must have outcome");
        assert!(check.get("detail").is_some(), "check must have detail");
    }

    let summary = &json["summary"];
    assert!(summary["pass"].is_u64(), "summary.pass must be numeric");
    assert!(summary["fail"].is_u64(), "summary.fail must be numeric");
    assert!(summary["skip"].is_u64(), "summary.skip must be numeric");

    assert!(
        !reborn_home.exists(),
        "doctor --json should not create state directories"
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_stdout_file_action(&stdout, "config.toml", "wrote");
    assert_stdout_file_action(&stdout, "providers.json", "wrote");
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
fn onboard_bootstraps_reborn_home_without_touching_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let v1_home = temp.path().join("v1-home");

    let output = reborn_command()
        .arg("onboard")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_BASE_DIR", &v1_home)
        .output()
        .expect("ironclaw-reborn onboard should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn onboarding"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("v1_state: not-used"), "stdout: {stdout}");
    assert!(
        reborn_home.join("config.toml").exists(),
        "config.toml missing"
    );
    assert!(
        reborn_home.join("providers.json").exists(),
        "providers.json missing"
    );
    // onboard also provisions a `<reborn_home>/webui-token` fallback file
    // so a service-installed `serve` (unit env carries only HOME/PROFILE)
    // still has a bearer token to read when IRONCLAW_REBORN_WEBUI_TOKEN is
    // unset.
    let webui_token_path = reborn_home.join("webui-token");
    assert!(webui_token_path.exists(), "webui-token file missing");
    let webui_token_text = std::fs::read_to_string(&webui_token_path).expect("read webui-token");
    assert!(
        webui_token_text.trim().len() >= 32,
        "generated webui-token must meet the >=32 byte entropy floor: {} bytes",
        webui_token_text.trim().len()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&webui_token_path)
            .expect("stat webui-token")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "webui-token file must be 0600, got {mode:o}");
    }
    assert_stdout_labeled_action(&stdout, "webui_token:", "wrote");
    let marker_path = reborn_home.join(".onboard-completed.json");
    assert!(marker_path.exists(), "onboarding marker missing");
    let marker_text = std::fs::read_to_string(marker_path).expect("read marker");
    let marker: serde_json::Value = serde_json::from_str(&marker_text).expect("valid marker JSON");
    assert_eq!(marker["schema_version"], "ironclaw.reborn.onboarding/v1");
    assert_eq!(marker["v1_state"], "not-used");
    assert!(
        !v1_home.exists(),
        "onboard must not create or read explicit v1 state"
    );
}

#[test]
fn onboard_is_idempotent_for_the_webui_token_file() {
    // The token doubles as `serve`'s session-signing key, so a re-run of
    // `onboard` must never clobber a valid existing token — that would
    // invalidate every signed session and any env var an operator copied
    // from the first run.
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let first = reborn_command()
        .arg("onboard")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("first onboard should run");
    assert!(
        first.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let token_path = reborn_home.join("webui-token");
    let first_token = std::fs::read_to_string(&token_path).expect("read webui-token");

    let second = reborn_command()
        .arg("onboard")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("second onboard should run");
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert_stdout_labeled_action(&second_stdout, "webui_token:", "preserved");
    let second_token = std::fs::read_to_string(&token_path).expect("read webui-token again");
    assert_eq!(
        first_token, second_token,
        "re-running onboard must not regenerate a valid webui-token"
    );
}

/// RED (B4 step 5, capstone): the full daemon-case journey through the real
/// binary — `onboard` non-interactively (so it soft-skips LLM-credential
/// prompts and OS-service install, matching a headless CI/first-boot
/// environment with no terminal), then `serve` spawned with `env_clear()`
/// and only `HOME`/`IRONCLAW_REBORN_HOME`/`IRONCLAW_DISABLE_OS_KEYCHAIN` —
/// no `IRONCLAW_REBORN_WEBUI_TOKEN`/`_USER_ID` overrides — must still bind.
/// `serve` reads the bearer from onboard's provisioned `webui-token` file
/// and the default owner id from `RebornHome`'s seeded config.
///
/// `config.toml`'s stub now seeds `nearai` — `commands/config/init.rs`'s
/// `DEFAULT_LLM_PROVIDER_ID`, shared with onboard's own interactive prompt
/// default, so the two paths can't drift apart — which carries
/// `"api_key_required": false` in `providers.json` (session-token auth, not
/// a bearer key). `serve` resolves the `[llm.default]` slot at startup, not
/// lazily on first `send_user_message` (`resolve_reborn_runtime_llm`, called
/// from `build_runtime_input_with_options` before the async runtime even
/// starts), so this test must boot against the stub exactly as onboard wrote
/// it — no config surgery. A key is still seeded through the same
/// `open_local_dev_secret_store` + `LlmKeyStore::put` opener
/// `provision_llm_credentials` uses (bypassing the prompt UI, matching
/// `provision_llm_credentials_writes_config_and_secret_store_through_fake_prompts`'s
/// seeding pattern) to exercise the stored-key overlay path a real
/// interactive onboarding run would also take; it is not required for
/// `serve` to boot since `api_key_required = false`.
/// This is the config+files-only daemon case `service install` produces: no
/// env vars carried through the launchd/systemd unit environment at all.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn onboard_then_serve_boots_with_an_empty_environment() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");

    let onboard_output = reborn_command()
        .arg("onboard")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");
    assert!(
        onboard_output.status.success(),
        "onboard must succeed non-interactively; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );
    let onboard_stdout = String::from_utf8_lossy(&onboard_output.stdout);
    assert!(
        onboard_stdout.contains("service: skipped (non-interactive session)"),
        "headless onboarding must not attempt a launchd/systemd install; stdout: {onboard_stdout}"
    );
    assert!(
        onboard_stdout.contains("login_link: http://127.0.0.1:3000/login?token="),
        "onboard must print the CLI-token login link; stdout: {onboard_stdout}"
    );
    assert!(
        reborn_home.join("webui-token").exists(),
        "onboard must provision the webui-token file `serve` reads as a fallback"
    );

    let config_path = reborn_home.join("config.toml");
    let config_text = std::fs::read_to_string(&config_path).expect("read seeded config.toml");
    assert!(
        config_text.contains("provider_id = \"nearai\""),
        "the config stub must default to the zero-friction nearai slot so serve boots headless \
         with no LLM env vars set: {config_text}"
    );

    // Land the LLM key the stub's `[llm.default]` can use (not required —
    // `api_key_required = false` — but seeded anyway to exercise the same
    // stored-key overlay path a real onboarding run would), through the same
    // opener onboard's own interactive path uses — no env var, no terminal.
    // `os_keychain_suppressed`'s `cfg!(test)` half only fires for
    // `ironclaw_secrets`'s OWN unit tests — evaluated against the crate being
    // COMPILED, not the caller — so from this integration-test binary
    // (`ironclaw_secrets` linked in as an ordinary, non-`cfg(test)`
    // dependency) it is false, and hitting the store opener without a cached
    // master key would fall through to a real macOS Keychain
    // `SecItemCopyMatching` call that hangs forever waiting on a GUI prompt
    // no headless run can answer (confirmed via `sample` on the wedged
    // process during this test's own development). Seed the cached dotfile
    // directly first — same pattern as
    // `provision_llm_credentials_writes_config_and_secret_store_through_fake_prompts`'s
    // unit test above — so the resolver never reaches the keychain step at
    // all, deterministically, regardless of `IRONCLAW_DISABLE_OS_KEYCHAIN`.
    std::fs::write(
        reborn_home.join(ironclaw_reborn_composition::LOCAL_DEV_SECRETS_MASTER_KEY_PATH),
        ironclaw_secrets::keychain::generate_master_key_hex(),
    )
    .expect("seed cached master key dotfile");
    // `new_current_thread`, not the multi-thread default: matches
    // `crate::runtime::block_on_cli_future`'s own runtime shape (the
    // production seam `provision_llm_credentials` runs under), which this
    // seeding step is standing in for outside a terminal. The libsql/rusqlite
    // path underneath `open_local_dev_secret_store` deadlocks when driven
    // from a plain `Runtime::new()` multi-thread runtime instead.
    let seed_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread tokio runtime for LLM key seed");
    seed_rt.block_on(async {
        let store = ironclaw_reborn_composition::open_local_dev_secret_store(&reborn_home)
            .await
            .expect("open local dev secret store");
        ironclaw_reborn_composition::LlmKeyStore::new(store)
            .put(
                "nearai",
                ironclaw_secrets::SecretMaterial::from("nearai-smoke-test-session".to_string()),
            )
            .await
            .expect("seed nearai key");
    });

    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    let _ = child.kill();
    let _ = child.wait();
}

/// Full-chain capstone: `onboard`'s printed CLI-token login link must
/// actually WORK once `serve` is up, and the session it mints must
/// authorize a real request against the WebChat v2 API — not just bind a
/// listener. Builds on `onboard_then_serve_boots_with_an_empty_environment`'s
/// setup (same seeding rationale — see that test's doc comment for why the
/// master-key dotfile and stored `nearai` key are seeded through
/// `seed_stored_llm_key` rather than env vars or a terminal prompt), then
/// drives the HTTP mechanics `serve_mounts_cli_login_route_without_sso`
/// already exercises (`GET /login?token=` → 302/303 with a `login_ticket` →
/// `POST /auth/session/exchange` → bearer) starting from onboard's OWN
/// provisioned token file instead of a hand-seeded one, and goes one step
/// further: the exchanged bearer is used to authenticate a real
/// `GET /api/webchat/v2/threads` call against the actually-composed
/// `webui_v2_app` (real `RebornServicesApi`, not a stub), asserting a
/// non-401/403 response — proving the login link's session is not just
/// mintable but usable.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn onboard_login_link_then_bearer_authorizes_a_protected_request() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");

    let onboard_output = reborn_command()
        .arg("onboard")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");
    assert!(
        onboard_output.status.success(),
        "onboard must succeed non-interactively; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );
    let onboard_stdout = String::from_utf8_lossy(&onboard_output.stdout);
    assert!(
        onboard_stdout.contains("login_link: http://127.0.0.1:3000/login?token="),
        "onboard must print the CLI-token login link; stdout: {onboard_stdout}"
    );
    let token_path = reborn_home.join("webui-token");
    assert!(
        token_path.exists(),
        "onboard must provision the webui-token file `serve` reads as a fallback"
    );
    let webui_token = std::fs::read_to_string(&token_path)
        .expect("read onboard-provisioned webui-token")
        .trim()
        .to_string();
    assert!(
        !webui_token.is_empty(),
        "onboard-provisioned webui-token must not be empty"
    );

    // Same stored-key seeding `onboard_then_serve_boots_with_an_empty_
    // environment` performs, through the shared `seed_stored_llm_key`
    // helper below — exercises the stored-key overlay path a real
    // interactive onboarding run would also take (not required for `serve`
    // to boot: the stub's `[llm.default]` is `nearai`, `api_key_required =
    // false`).
    seed_stored_llm_key(&reborn_home, "nearai", "nearai-smoke-test-session");

    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    // 1. The onboard-provisioned token, presented at `/login?token=`,
    //    must redirect into the ticket hand-off — this is the exact URL
    //    `onboard` printed as `login_link:` above (minus the placeholder
    //    port).
    let login = http_response(
        port,
        &format!(
            "GET /login?token={webui_token} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        ),
        "onboard login-link probe",
    );
    let login = match login {
        Ok(response) => response,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("login-link probe failed: {error}");
        }
    };
    assert!(
        login.status_line.contains(" 302 ") || login.status_line.contains(" 303 "),
        "onboard's login link must redirect into the ticket hand-off, got: {}",
        login.status_line
    );
    let location = login
        .header("location")
        .expect("redirect must carry a Location header");
    assert!(
        location.starts_with("/v2?login_ticket="),
        "redirect must land on the SPA with a login_ticket, got: {location}"
    );
    let ticket = location
        .split("login_ticket=")
        .nth(1)
        .expect("redirect Location must carry a login_ticket query param");

    // 2. Exchange the ticket for the real session bearer.
    let exchange_body = format!(r#"{{"ticket":"{ticket}"}}"#);
    let exchange_request = format!(
        "POST /auth/session/exchange HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{exchange_body}",
        exchange_body.len()
    );
    let exchange = http_response(port, &exchange_request, "session exchange probe");
    let exchange = match exchange {
        Ok(response) => response,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("session exchange probe failed: {error}");
        }
    };
    assert!(
        exchange.status_line.contains(" 200 "),
        "ticket must exchange for a real bearer, got: {}; body: {}",
        exchange.status_line,
        exchange.body
    );
    #[derive(serde::Deserialize)]
    struct ExchangeResponse {
        token: String,
    }
    let bearer: ExchangeResponse =
        serde_json::from_str(&exchange.body).expect("exchange response body must be valid JSON");
    assert!(
        !bearer.token.is_empty(),
        "exchanged bearer must not be empty"
    );

    // 3. The exchanged bearer must authorize a REAL request against the
    //    composed WebChat v2 API surface (production `RebornServicesApi`,
    //    the same one `serve` wires for real traffic) — not merely be a
    //    well-formed token. A regression that mints a bearer the auth
    //    middleware then rejects (or that never reaches the real facade)
    //    would slip past a test that stops at the exchange response.
    let api_request = format!(
        "GET /api/webchat/v2/threads HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
        bearer.token
    );
    let api_response = http_response(port, &api_request, "authenticated protected-route probe");

    let _ = child.kill();
    let _ = child.wait();

    let api_response = api_response.expect("authenticated protected-route probe must complete");
    assert!(
        !api_response.status_line.contains(" 401 ") && !api_response.status_line.contains(" 403 "),
        "the login link's exchanged bearer must authorize a real WebChat v2 request, got: {}; body: {}",
        api_response.status_line,
        api_response.body
    );
    assert!(
        api_response.status_line.contains(" 200 "),
        "GET /api/webchat/v2/threads with a valid bearer should succeed, got: {}; body: {}",
        api_response.status_line,
        api_response.body
    );
}

/// Seed the local-dev encrypted secret store with an LLM API key for
/// `provider_id`, through the same `open_local_dev_secret_store` +
/// `LlmKeyStore::put` opener `onboard`'s interactive credential prompt uses
/// — bypassing the prompt UI, matching
/// `onboard_then_serve_boots_with_an_empty_environment`'s own seeding
/// pattern. Also seeds the cached master-key dotfile first so the resolver
/// never reaches the OS keychain (see that test's comment for why: a
/// headless run would otherwise hang on a GUI keychain prompt).
#[cfg(feature = "webui-v2-beta")]
fn seed_stored_llm_key(reborn_home: &Path, provider_id: &str, key: &str) {
    std::fs::write(
        reborn_home.join(ironclaw_reborn_composition::LOCAL_DEV_SECRETS_MASTER_KEY_PATH),
        ironclaw_secrets::keychain::generate_master_key_hex(),
    )
    .expect("seed cached master key dotfile");
    let seed_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread tokio runtime for LLM key seed");
    let provider_id = provider_id.to_string();
    let key = key.to_string();
    let reborn_home = reborn_home.to_path_buf();
    seed_rt.block_on(async move {
        let store = ironclaw_reborn_composition::open_local_dev_secret_store(&reborn_home)
            .await
            .expect("open local dev secret store");
        ironclaw_reborn_composition::LlmKeyStore::new(store)
            .put(&provider_id, ironclaw_secrets::SecretMaterial::from(key))
            .await
            .expect("seed provider key");
    });
}

/// REGRESSION (review comments #4/#6): before the fix,
/// `build_runtime_input_with_options` called `resolve_reborn_runtime_llm`
/// directly, which fails closed on `ApiKeyEnvUnset` for an
/// `api_key_required = true` provider (openai/anthropic) *before*
/// `apply_startup_stored_llm_key` ever gets a chance to inject the key an
/// operator stored through `onboard`/`models set-provider`. `onboard` itself
/// never fails (it doesn't resolve the runtime LLM, just prompts and
/// stores), so the bug only surfaced at the next `serve` boot — silently
/// stranding an operator who had just "successfully" onboarded. `nearai`
/// (`api_key_required = false`) never hit this path, which is why the
/// existing daemon-case capstone (`onboard_then_serve_boots_with_an_empty_
/// environment`) didn't catch it.
///
/// Crate smoke tier (spawns the real `ironclaw-reborn` binary), matching
/// `onboard_then_serve_boots_with_an_empty_environment`'s tier: the bug lives
/// in `serve`'s pre-async-runtime boot sequence
/// (`build_runtime_input_with_options`, called before `build_reborn_services`
/// even starts), so only a real-process boot proves the ordering is fixed —
/// an in-process integration-tier test would have to reconstruct that same
/// boot sequence and couldn't observe the actual "does the process bind"
/// outcome any more directly than spawning it does.
///
/// Also closes a coverage gap: the model an operator scripts through
/// `models set-provider --model <model>` (the non-interactive equivalent of
/// onboard's own model prompt — see the comment below) must be the model
/// `serve` actually resolves for the runtime, not just A model. Asserted
/// via the `debug!` trace `build_runtime_input_with_options` already emits
/// at `runtime/mod.rs:676-680` (`"resolved LLM selection for Reborn
/// runtime"`, fields `provider_id`/`model`) once scoped into view with
/// `IRONCLAW_REBORN_LOG` (the crate's own env knob — see
/// `runtime::init_tracing` — not `RUST_LOG`, and never `info!`/`warn!` for
/// this per the REPL/TUI logging-level rule: `debug!` only). A deliberately
/// non-default model name (`gpt-test-model`, distinct from `openai`'s
/// catalog default) proves the resolved value flows all the way from the
/// scripted answer, not a hardcoded fallback.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn onboard_openai_key_then_serve_boots_with_env_var_unset() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");

    let onboard_output = reborn_command()
        .arg("onboard")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");
    assert!(
        onboard_output.status.success(),
        "onboard must succeed non-interactively; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );

    // Point `[llm.default]` at `openai` (api_key_required = true) with a
    // deliberately non-default model name, the same way an operator who ran
    // `onboard`'s interactive credential prompt (which also asks for a
    // model) would have ended up configured — `models set-provider` is the
    // non-interactive equivalent this test drives instead of a terminal.
    const SCRIPTED_MODEL: &str = "gpt-test-model";
    let set_provider_output = reborn_command()
        .args([
            "models",
            "set-provider",
            "openai",
            "--model",
            SCRIPTED_MODEL,
        ])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set-provider should run");
    assert!(
        set_provider_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&set_provider_output.stderr)
    );

    // Store the key the same way `onboard`'s interactive prompt would have
    // — never in config.toml or the environment.
    seed_stored_llm_key(&reborn_home, "openai", "sk-smoke-test-stored-openai-key");

    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        // No OPENAI_API_KEY: the stored key must be what makes this boot.
        // Scoped to this crate (not a blanket `debug`) so the resolved-LLM
        // trace is observable without flooding stderr with third-party
        // crate noise `protect_reborn_log_filter` would otherwise still
        // clamp anyway.
        // Target is `ironclaw_reborn::runtime`: `tracing`'s default target
        // is the compiled crate name, which for the `ironclaw-reborn`
        // binary target (no separate lib crate) is the dash-to-underscore
        // normalized BIN name — `ironclaw_reborn` — not the Cargo package
        // name `ironclaw_reborn_cli` this test crate itself is compiled as.
        .env("IRONCLAW_REBORN_LOG", "info,ironclaw_reborn=debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    let pre_banner_stderr = strip_ansi(&wait_for_serve_banner(&mut child));

    let _ = child.kill();
    let _ = child.wait();

    assert!(
        pre_banner_stderr.contains("resolved LLM selection for Reborn runtime"),
        "serve must emit the resolved-LLM debug trace before binding; stderr: {pre_banner_stderr}"
    );
    assert!(
        pre_banner_stderr.contains("provider_id=openai"),
        "resolved-LLM trace must name the openai provider; stderr: {pre_banner_stderr}"
    );
    assert!(
        pre_banner_stderr.contains(&format!("model={SCRIPTED_MODEL}")),
        "resolved-LLM trace must carry the scripted model `{SCRIPTED_MODEL}`, proving the \
         operator's `models set-provider --model` answer reached the runtime `serve` actually \
         boots with, not a hardcoded default; stderr: {pre_banner_stderr}"
    );
}

/// RAILWAY PIN 1: the production Railway deployment boots with an
/// `api_key_required = false` provider (`nearai`) and its API key env var
/// (`NEARAI_API_KEY`) set — never a stored key, never an unset-key error.
/// This pins that the stored-key fallback added for the regression above is
/// additive only: the ordinary env-var-set boot path must behave exactly as
/// before, with the secret store never even opened (an empty store, as
/// Railway's is, must not matter here — it is only ever consulted on the
/// `ApiKeyEnvUnset` error path, which this scenario never reaches).
#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_boots_with_env_api_key_set_and_empty_secret_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");

    let onboard_output = reborn_command()
        .arg("onboard")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");
    assert!(
        onboard_output.status.success(),
        "onboard must succeed non-interactively; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );
    // Sanity: the config stub defaults to `nearai`, matching Railway's
    // deployment shape, and the secret store is never touched by this test.
    let config_text =
        std::fs::read_to_string(reborn_home.join("config.toml")).expect("read seeded config.toml");
    assert!(
        config_text.contains("provider_id = \"nearai\""),
        "config: {config_text}"
    );

    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("NEARAI_API_KEY", "railway-shape-smoke-test-nearai-key")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    let _ = child.kill();
    let _ = child.wait();
}

/// RAILWAY PIN 2: an `api_key_required = true` provider with neither the env
/// var set nor a key in the secret store must still fail closed at boot with
/// the same `ApiKeyEnvUnset` error text as before this fix — the
/// stored-key fallback must never mask a genuine misconfiguration.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_fails_closed_when_neither_env_nor_store_has_the_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");

    let onboard_output = reborn_command()
        .arg("onboard")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");
    assert!(
        onboard_output.status.success(),
        "onboard must succeed non-interactively; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );
    let set_provider_output = reborn_command()
        .args(["models", "set-provider", "openai", "--model", "gpt-5-mini"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set-provider should run");
    assert!(
        set_provider_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&set_provider_output.stderr)
    );
    // No `seed_stored_llm_key` call: the secret store stays empty.

    // Not a blocking `.output()`: this test asserts an *expected* fail-closed
    // exit, so a regression that makes `serve` bind a listener instead of
    // exiting would otherwise hang this test (and CI) forever waiting on
    // process exit. Spawn, poll `try_wait()` against a deadline, and kill on
    // timeout instead.
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port", "0"])
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        // No OPENAI_API_KEY set.
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");

    // Drain stdout/stderr on background threads while polling for exit, so a
    // full pipe buffer can never block the child from exiting (and thus
    // block `try_wait()` from ever observing that exit).
    let mut stdout_reader = child.stdout.take().expect("stdout should be piped");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout_reader, &mut buf);
        buf
    });
    let mut stderr_reader = child.stderr.take().expect("stderr should be piped");
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr_reader, &mut buf);
        buf
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().expect("serve child status") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "serve did not exit within the deadline; expected a fail-closed exit with \
                 neither an env key nor a stored key — it may have regressed into binding a \
                 listener instead"
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };

    let stdout_bytes = stdout_thread.join().expect("stdout reader thread panicked");
    let stderr_bytes = stderr_thread.join().expect("stderr reader thread panicked");
    let stderr = String::from_utf8_lossy(&stderr_bytes);

    assert!(
        !status.success(),
        "serve must fail closed with neither an env key nor a stored key; stdout: {}",
        String::from_utf8_lossy(&stdout_bytes)
    );
    assert!(
        stderr
            .contains("llm provider `openai` requires API key env var `OPENAI_API_KEY` to be set"),
        "stderr must carry the same ApiKeyEnvUnset error text as before this fix: {stderr}"
    );
}

/// Security fix companion: `onboard`'s finale must not print a CLI-token
/// login link when the operator's env var is active — that link would point
/// at a route `serve` no longer mounts for an env-sourced token (see
/// `serve_does_not_mount_cli_login_route_when_token_is_env_sourced`). It
/// must instead note that the env token is in charge.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn onboard_prints_env_token_note_instead_of_login_link_when_env_token_is_set() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");

    let onboard_output = reborn_command()
        .arg("onboard")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "reborn-smoke-test-onboard-env-token-0123456789abcdef",
        )
        .output()
        .expect("ironclaw-reborn onboard should run");
    assert!(
        onboard_output.status.success(),
        "onboard must succeed non-interactively; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&onboard_output.stdout);
    assert!(
        stdout.contains("login_note: IRONCLAW_REBORN_WEBUI_TOKEN is set"),
        "stdout must note the active env var instead of a login link: {stdout}"
    );
    assert!(
        !stdout.contains("login_link:"),
        "stdout must not print a login link that points at an unmounted route: {stdout}"
    );
}

/// `status` companion to the onboard test above: reprinting the login link
/// after onboarding must also respect the env-token precedence — *unless*
/// `status` already knows the OS service isn't running, in which case that
/// takes priority (see `commands::status::apply_service_suppression`):
/// there is no point advertising any login credential, env-sourced or not,
/// into a `serve` process that demonstrably isn't listening.
///
/// The service-state query is deliberately host-wide (`launchctl list` /
/// `systemctl show`), not scoped to this test's temp `$HOME` — there is
/// only one system service manager. So this test can't pin an exact
/// `service:` value: on a clean CI runner it reads "not installed"; on a
/// dev host that happens to have the real `ironclaw-reborn` service
/// registered (including mid crash-loop — the bug this fix addresses) it
/// may read "stopped" or even transiently "running". Assert the
/// *invariant* instead — `login_link` is always absent, and `login_note`
/// matches whichever branch the observed `service:` state took — rather
/// than a specific state.
#[cfg(feature = "webui-v2-beta")]
#[test]
fn status_prints_env_token_note_instead_of_login_link_when_env_token_is_set() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");

    let onboard_output = reborn_command()
        .arg("onboard")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");
    assert!(
        onboard_output.status.success(),
        "onboard must succeed non-interactively; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );

    let status_output = reborn_command()
        .arg("status")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "reborn-smoke-test-status-env-token-0123456789abcdef",
        )
        .output()
        .expect("ironclaw-reborn status should run");
    assert!(
        status_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&status_output.stderr)
    );
    let stdout = String::from_utf8_lossy(&status_output.stdout);
    let service_line = stdout
        .lines()
        .find(|line| line.starts_with("service:"))
        .unwrap_or_else(|| panic!("status text must include a `service:` line: {stdout}"));
    let service_state = service_line.trim_start_matches("service:").trim();
    match service_state {
        "running" => assert!(
            stdout.contains("login_note:") && stdout.contains("IRONCLAW_REBORN_WEBUI_TOKEN is set"),
            "service is genuinely running, so the env-token note must still win: {stdout}"
        ),
        "stopped" | "not installed" => assert!(
            stdout.contains("login_note:") && stdout.contains("service is not running"),
            "a known not-running service must take priority over the env-token note — \
             there is no login credential (env-sourced or not) worth advertising into \
             a `serve` process that isn't listening: {stdout}"
        ),
        other => panic!("unexpected service: state {other:?}: {stdout}"),
    }
    assert!(
        !stdout.contains("login_link:"),
        "stdout must not print a login link that points at an unmounted route \
         (a valid webui-token file exists from onboard, but the env var takes \
         precedence): {stdout}"
    );
}

#[test]
fn onboard_dry_run_is_read_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = reborn_command()
        .args(["onboard", "--dry-run", "--import-history"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard --dry-run should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn onboarding dry run"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("import_history_requested: true"),
        "stdout: {stdout}"
    );
    assert!(!reborn_home.exists(), "dry-run must not create Reborn home");
}

#[test]
fn onboard_dry_run_reports_existing_marker_as_preserved() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    let marker_path = reborn_home.join(".onboard-completed.json");
    std::fs::write(&marker_path, "custom marker\n").expect("write marker");

    let output = reborn_command()
        .args(["onboard", "--dry-run"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard --dry-run should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!("would_preserve: {}", marker_path.display())),
        "stdout: {stdout}"
    );
    let marker_text = std::fs::read_to_string(marker_path).expect("read marker");
    assert_eq!(marker_text, "custom marker\n");
}

#[test]
fn onboard_dry_run_propagates_a_webui_token_io_error_without_mutating_home() {
    // `print_dry_run` propagates `webui_token_file_is_valid`'s error with
    // `?` instead of defaulting to "would_write" on an I/O failure (see
    // that fn's doc comment). Pin the end-to-end behavior: a directory
    // planted at the token path is a real I/O error, the process exits
    // non-zero, and the dry run's read-only contract still holds — no
    // marker or config file gets written to the (already-existing)
    // Reborn home.
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir reborn_home");
    std::fs::create_dir_all(reborn_home.join("webui-token"))
        .expect("seed a directory at token path");

    let output = Command::new(reborn_bin())
        .args(["onboard", "--dry-run"])
        .env_clear()
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard --dry-run should run");

    assert!(
        !output.status.success(),
        "a directory at the token path must fail dry-run, not silently proceed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !reborn_home.join(".onboard-completed.json").exists(),
        "a failed dry-run must not write the onboarding marker"
    );
    assert!(
        !reborn_home.join("config.toml").exists(),
        "a failed dry-run must not write config.toml"
    );
    assert!(
        std::fs::metadata(reborn_home.join("webui-token"))
            .expect("token path still present")
            .is_dir(),
        "a failed dry-run must not touch the pre-existing token path"
    );
}

#[test]
fn onboard_import_history_records_pending_step() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = reborn_command()
        .args(["onboard", "--import-history"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard --import-history should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let marker_text =
        std::fs::read_to_string(reborn_home.join(".onboard-completed.json")).expect("read marker");
    let marker: serde_json::Value = serde_json::from_str(&marker_text).expect("valid marker JSON");
    let pending = marker["steps_pending"]
        .as_array()
        .expect("pending steps array");
    assert!(
        pending.iter().any(|step| step == "history_import"),
        "marker should record history import as pending: {marker_text}"
    );
}

#[test]
fn onboard_preserves_existing_config_without_force() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(reborn_home.join("config.toml"), "custom config\n").expect("write config");
    std::fs::write(
        reborn_home.join(".onboard-completed.json"),
        "custom marker\n",
    )
    .expect("write marker");

    let output = reborn_command()
        .arg("onboard")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_stdout_file_action(&stdout, "config.toml", "preserved");
    assert_stdout_file_action(&stdout, "providers.json", "wrote");
    assert_stdout_labeled_action(&stdout, "onboarding_marker:", "preserved");
    let config_text =
        std::fs::read_to_string(reborn_home.join("config.toml")).expect("read config");
    assert_eq!(config_text, "custom config\n");
    let marker_text =
        std::fs::read_to_string(reborn_home.join(".onboard-completed.json")).expect("read marker");
    assert_eq!(marker_text, "custom marker\n");
    assert!(
        reborn_home.join("providers.json").exists(),
        "missing providers file"
    );
    assert!(
        reborn_home.join(".onboard-completed.json").exists(),
        "missing marker"
    );
}

#[test]
fn onboard_with_force_overwrites_existing_files_and_marker() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(reborn_home.join("config.toml"), "custom config\n").expect("write config");
    std::fs::write(reborn_home.join("providers.json"), "custom providers\n")
        .expect("write providers");
    std::fs::write(
        reborn_home.join(".onboard-completed.json"),
        "custom marker\n",
    )
    .expect("write marker");

    let output = reborn_command()
        .args(["onboard", "--force"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard --force should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_stdout_file_action(&stdout, "config.toml", "overwrote");
    assert_stdout_file_action(&stdout, "providers.json", "overwrote");
    assert_stdout_labeled_action(&stdout, "onboarding_marker:", "overwrote");

    let config_text =
        std::fs::read_to_string(reborn_home.join("config.toml")).expect("read config");
    let providers_text =
        std::fs::read_to_string(reborn_home.join("providers.json")).expect("read providers");
    let marker_text =
        std::fs::read_to_string(reborn_home.join(".onboard-completed.json")).expect("read marker");
    assert!(!config_text.contains("custom config"));
    assert!(!providers_text.contains("custom providers"));
    assert!(!marker_text.contains("custom marker"));
    assert!(config_text.contains("api_version = \"ironclaw.runtime/v1\""));
    assert!(providers_text.contains("\"id\": \"acme-openrouter\""));
    let marker: serde_json::Value = serde_json::from_str(&marker_text).expect("valid marker JSON");
    assert_eq!(marker["schema_version"], "ironclaw.reborn.onboarding/v1");
}

/// `onboard` provisions a local-dev secrets master key: with no cached
/// `.reborn-local-dev-secrets-master-key` dotfile and no OS-keychain entry,
/// it generates one and stores it in the keychain. This CI/test spawn (a
/// real compiled binary, not `cargo test`'s `cfg!(test)`) sets
/// `IRONCLAW_DISABLE_OS_KEYCHAIN=1`, which suppresses that OS keychain write
/// (`ironclaw_secrets::keychain::store_master_key` fails closed under
/// suppression — see `crates/ironclaw_secrets/src/keychain.rs`), so this
/// pins the "headless Linux / denied prompt" fallback path specifically: the
/// command must print the `SECRETS_MASTER_KEY`/dotfile fallback note and
/// still exit 0, never fail onboarding just because the keychain step
/// provisioned nothing. Verifying an actual successful keychain write needs
/// a real OS keychain and is manual/E2E only (not unit-testable in CI).
#[test]
fn onboard_reports_suppressed_master_key_fallback_and_still_succeeds() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = reborn_command()
        .arg("onboard")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");

    assert!(
        output.status.success(),
        "onboard must exit 0 even when the OS keychain is suppressed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("master_key: OS keychain unavailable; falling back to env/dotfile"),
        "stdout must report the suppressed keychain outcome: {stdout}"
    );
    assert!(
        stdout.contains("master_key_note:") && stdout.contains("SECRETS_MASTER_KEY"),
        "stdout must print the SECRETS_MASTER_KEY/dotfile fallback note: {stdout}"
    );
    // The suppressed keychain path must not itself create the dotfile —
    // that remains the resolver's own auto-gen-on-first-boot job (B1's
    // `resolve_local_dev_secret_master_key_with_env`), not onboarding's.
    assert!(
        !reborn_home
            .join(".reborn-local-dev-secrets-master-key")
            .exists(),
        "onboard's suppressed keychain path must not write the dotfile itself"
    );
}

/// A second `onboard` run over the same Reborn home must not attempt the
/// keychain again once a cached dotfile exists (from a prior `serve`/onboard
/// boot) — it's a no-op that reports `cached dotfile already present`.
#[test]
fn onboard_master_key_provisioning_is_a_noop_once_a_dotfile_is_cached() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join(".reborn-local-dev-secrets-master-key"),
        "a".repeat(64),
    )
    .expect("seed cached master-key dotfile");

    let output = reborn_command()
        .arg("onboard")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn onboard should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("master_key: cached dotfile already present"),
        "stdout must report the dotfile no-op: {stdout}"
    );
    let dotfile_text =
        std::fs::read_to_string(reborn_home.join(".reborn-local-dev-secrets-master-key"))
            .expect("read cached dotfile");
    assert_eq!(
        dotfile_text,
        "a".repeat(64),
        "an existing cached dotfile must not be rewritten by onboard"
    );
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

// ─── status ───────────────────────────────────────────────────────────────

#[test]
fn status_reports_reborn_home_without_touching_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .arg("status")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn status should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn status"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(reborn_home.to_str().expect("utf8 path")),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("local-dev"), "stdout: {stdout}");
    assert!(stdout.contains("text_only"), "stdout: {stdout}");
    assert!(
        !stdout.contains("v1_state"),
        "status output should not include v1_state"
    );
    assert!(
        !reborn_home.exists(),
        "status should not create state directories"
    );
}

#[test]
fn status_json_reports_reborn_home_without_touching_v1_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .arg("status")
        .arg("--json")
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn status --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(
        json["reborn_home"],
        reborn_home.to_str().expect("utf8 path")
    );
    assert_eq!(json["profile"], "local-dev");
    assert!(json["drivers"]["text_only"].is_object());
    assert!(
        json.get("v1_state").is_none(),
        "status JSON should not include v1_state"
    );
    assert!(
        !reborn_home.exists(),
        "status should not create state directories"
    );
}

#[test]
fn status_json_reports_present_config_and_providers_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("create Reborn home");
    let config_path = reborn_home.join("config.toml");
    let providers_path = reborn_home.join("providers.json");
    std::fs::write(&config_path, "").expect("write config");
    std::fs::write(&providers_path, "[]").expect("write providers");

    let output = Command::new(reborn_bin())
        .args(["status", "--json"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn status --json should run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid status JSON");
    assert_eq!(
        json["config_file"]["path"],
        config_path.to_str().expect("utf8")
    );
    assert_eq!(json["config_file"]["present"], true);
    assert_eq!(
        json["providers_file"]["path"],
        providers_path.to_str().expect("utf8")
    );
    assert_eq!(json["providers_file"]["present"], true);
}

// ─── config list ──────────────────────────────────────────────────────────

#[test]
fn config_list_reports_entries_without_creating_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .args(["config", "list"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn config list should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("IronClaw Reborn config"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("api_version"), "stdout: {stdout}");
    assert!(stdout.contains("boot.profile"), "stdout: {stdout}");
    assert!(stdout.contains("harness.id"), "stdout: {stdout}");
    assert!(
        stdout.contains("llm.default.provider_id"),
        "stdout: {stdout}"
    );
    assert!(
        !reborn_home.exists(),
        "config list should not create state directories"
    );
}

#[test]
fn config_list_json_reports_entries_without_creating_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .args(["config", "list", "--json"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn config list --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    let entries = json["entries"].as_array().expect("entries is array");
    assert!(!entries.is_empty(), "entries should not be empty");
    let first = &entries[0];
    assert!(first.get("key").is_some(), "entry should have key field");
    assert!(
        entries
            .iter()
            .any(|e| e["key"] == "llm.default.provider_id"),
        "entries should include llm.default.provider_id"
    );
    assert!(
        !reborn_home.exists(),
        "config list should not create state directories"
    );
}

// ─── config get ───────────────────────────────────────────────────────────

#[test]
fn config_get_known_key_prints_value() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .args(["config", "get", "boot.profile"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn config get should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("(not set)") || stdout.contains("local-dev"),
        "stdout should contain the value or (not set): {stdout}"
    );
}

#[test]
fn config_get_known_key_json_prints_value() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .args(["config", "get", "boot.profile", "--json"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn config get --json should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
    assert_eq!(json["key"], "boot.profile");
}

#[test]
fn config_get_unknown_key_exits_nonzero() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = Command::new(reborn_bin())
        .args(["config", "get", "nonexistent.key"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .output()
        .expect("ironclaw-reborn config get should run");

    assert!(
        !output.status.success(),
        "config get with unknown key should exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown config key"),
        "stderr should mention unknown key: {stderr}"
    );
}

#[test]
fn config_list_rejects_malformed_config() {
    assert_config_read_rejects_malformed(&["config", "list"]);
}

#[test]
fn config_get_rejects_malformed_config() {
    assert_config_read_rejects_malformed(&["config", "get", "boot.profile"]);
}

fn assert_config_read_rejects_malformed(args: &[&str]) {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("create Reborn home");
    std::fs::write(reborn_home.join("config.toml"), "[boot\nprofile = broken")
        .expect("write malformed config");

    let output = Command::new(reborn_bin())
        .args(args)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("config read command should run");
    assert!(!output.status.success(), "malformed config must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to parse") || stderr.contains("TOML"),
        "stderr should report parse failure: {stderr}"
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

    let output = isolated_no_llm_command(temp.path(), &reborn_home)
        .args(["run", "-m", "ping"])
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

#[cfg(feature = "root-llm-provider")]
#[test]
fn run_warns_when_falling_back_to_stub_gateway() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");

    let output = isolated_no_llm_command(&workspace, &reborn_home)
        .args(["run", "-m", "ping"])
        .output()
        .expect("ironclaw-reborn run should not crash");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no LLM selection configured") && stderr.contains("Runs will fail"),
        "stderr should warn about degraded stub-gateway boot; got: {stderr}"
    );
    assert!(
        reborn_home
            .join("local-dev/system/skills/code-review/SKILL.md")
            .is_file(),
        "runtime bootstrap should install bundled Reborn skills"
    );
}

#[test]
fn run_confirm_host_access_flag_gates_local_dev_yolo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let missing = local_yolo_command(&temp, &["run", "-m", "ping"])
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(!missing.status.success(), "missing confirmation must fail");
    let missing_stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        missing_stderr.contains("requires explicit disclosure acknowledgement"),
        "stderr should require disclosure acknowledgement; got: {missing_stderr}"
    );
    assert!(
        !reborn_home.join("config.toml").exists(),
        "failed host-access preflight must not seed runtime config"
    );

    let confirmed = local_yolo_command(&temp, &["run", "--confirm-host-access", "-m", "ping"])
        .output()
        .expect("ironclaw-reborn run should not crash");
    let confirmed_stderr = String::from_utf8_lossy(&confirmed.stderr);
    assert!(
        !confirmed_stderr.contains("requires explicit disclosure acknowledgement")
            && !confirmed_stderr.contains("requires --confirm-host-access"),
        "confirmed run should pass the host-access gate; got: {confirmed_stderr}"
    );
    let config = std::fs::read_to_string(reborn_home.join("config.toml"))
        .expect("confirmed first runtime start should seed config");
    assert!(
        config.contains("profile = \"local-dev\""),
        "env-selected local-dev-yolo must not become the persistent default: {config}"
    );
}

#[test]
fn run_confirm_host_access_requires_home_or_userprofile() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("reborn home");

    let output = reborn_command()
        .args(["run", "--confirm-host-access", "-m", "ping"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_REBORN_PROFILE", "local-dev-yolo")
        .output()
        .expect("ironclaw-reborn run should not crash");

    assert!(!output.status.success(), "missing host home must fail"); // safety: test-only assertion.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        /* safety: test-only assertion. */
        stderr.contains("HOME or USERPROFILE must be set"),
        "stderr should require a host home root; got: {stderr}"
    );
}

#[test]
fn run_confirm_host_access_uses_userprofile_when_home_is_absent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let host_home = temp.path().join("host-home");
    std::fs::create_dir_all(&reborn_home).expect("reborn home");
    std::fs::create_dir_all(&host_home).expect("host home");

    let output = reborn_command()
        .args(["run", "--confirm-host-access", "-m", "ping"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_REBORN_PROFILE", "local-dev-yolo")
        .env("USERPROFILE", &host_home)
        .output()
        .expect("ironclaw-reborn run should not crash");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("HOME or USERPROFILE must be set")
            && !stderr.contains("requires explicit disclosure acknowledgement")
            && !stderr.contains("requires --confirm-host-access"),
        "confirmed run should use USERPROFILE and pass the host-access gate; got: {stderr}"
    );
}

#[test]
fn repl_confirm_host_access_flag_gates_local_dev_yolo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let missing = local_yolo_command(&temp, &["repl"])
        .stdin(Stdio::null())
        .output()
        .expect("ironclaw-reborn repl should not crash");
    assert!(!missing.status.success(), "missing confirmation must fail");
    let missing_stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        missing_stderr.contains("requires explicit disclosure acknowledgement"),
        "stderr should require disclosure acknowledgement; got: {missing_stderr}"
    );

    let confirmed = local_yolo_command(&temp, &["repl", "--confirm-host-access"])
        .stdin(Stdio::null())
        .output()
        .expect("ironclaw-reborn repl should not crash");
    let confirmed_stderr = String::from_utf8_lossy(&confirmed.stderr);
    assert!(
        !confirmed_stderr.contains("requires explicit disclosure acknowledgement")
            && !confirmed_stderr.contains("requires --confirm-host-access"),
        "confirmed repl should pass the host-access gate; got: {confirmed_stderr}"
    );
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_confirm_host_access_flag_gates_local_dev_yolo() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let missing = local_yolo_command(&temp, &["serve"])
        .output()
        .expect("ironclaw-reborn serve should not crash");
    assert!(!missing.status.success(), "missing confirmation must fail");
    let missing_stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(
        missing_stderr.contains("requires explicit disclosure acknowledgement"),
        "stderr should require disclosure acknowledgement; got: {missing_stderr}"
    );
    assert!(
        !reborn_home.join("config.toml").exists(),
        "failed host-access preflight must not seed runtime config"
    );

    let confirmed = local_yolo_command(&temp, &["serve", "--confirm-host-access"])
        .output()
        .expect("ironclaw-reborn serve should not crash");
    assert!(
        !confirmed.status.success(),
        "serve still needs webui token config"
    );
    let confirmed_stderr = String::from_utf8_lossy(&confirmed.stderr);
    assert!(
        !confirmed_stderr.contains("requires explicit disclosure acknowledgement")
            && !confirmed_stderr.contains("requires --confirm-host-access"),
        "confirmed serve should pass the host-access gate; got: {confirmed_stderr}"
    );
    assert!(
        !reborn_home.join("config.toml").exists(),
        "failed WebUI token preflight must not seed runtime config"
    );
    assert!(
        confirmed_stderr.contains("IRONCLAW_REBORN_WEBUI_TOKEN"),
        "confirmed serve should reach WebUI token resolution; got: {confirmed_stderr}"
    );
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_confirmed_local_dev_yolo_rejects_non_loopback_cli_host() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = local_yolo_command(
        &temp,
        &["serve", "--confirm-host-access", "--host", "0.0.0.0"],
    )
    .env(
        "IRONCLAW_REBORN_WEBUI_TOKEN",
        // >=32 bytes: serve now enforces the session-signing entropy
        // floor unconditionally (it signs admin-minted session tokens
        // even without SSO).
        "reborn-smoke-test-token-0123456789abcdef",
    )
    .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
    .output()
    .expect("ironclaw-reborn serve should not crash");

    assert!(
        !output.status.success(),
        "non-loopback confirmed yolo serve must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refuses non-loopback listener 0.0.0.0")
            && stderr.contains("trusted-laptop host access"),
        "stderr should reject non-loopback trusted-laptop access; got: {stderr}"
    );
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_confirmed_local_dev_yolo_rejects_non_loopback_config_host() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("reborn home");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[webui]
listen_host = "0.0.0.0"
"#,
    )
    .expect("write config");

    let output = local_yolo_command(&temp, &["serve", "--confirm-host-access"])
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            // >=32 bytes: serve now enforces the session-signing entropy
            // floor unconditionally (it signs admin-minted session tokens
            // even without SSO).
            "reborn-smoke-test-token-0123456789abcdef",
        )
        .env("IRONCLAW_REBORN_WEBUI_USER_ID", "test-user")
        .output()
        .expect("ironclaw-reborn serve should not crash");

    assert!(
        !output.status.success(),
        "non-loopback confirmed yolo serve from config must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refuses non-loopback listener 0.0.0.0")
            && stderr.contains("trusted-laptop host access"),
        "stderr should reject config-driven non-loopback trusted-laptop access; got: {stderr}"
    );
}

#[cfg(feature = "webui-v2-beta")]
#[test]
fn serve_local_dev_allows_non_loopback_without_trusted_laptop_access() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = Command::new(reborn_bin())
        .args(["serve", "--host", "0.0.0.0", "--port", "0"])
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .env_remove("IRONCLAW_REBORN_WEBUI_TOKEN")
        .env_remove("IRONCLAW_REBORN_WEBUI_USER_ID")
        .output()
        .expect("ironclaw-reborn serve should not crash");

    assert!(
        !output.status.success(),
        "serve should still fail closed on missing WebUI token"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_WEBUI_TOKEN must be set"),
        "ordinary local-dev serve should reach WebUI token validation; got: {stderr}"
    );
    assert!(
        !stderr.contains("trusted-laptop host access"),
        "ordinary local-dev serve should not trigger the trusted-laptop listener refusal; got: {stderr}"
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

    let output = isolated_no_llm_command(temp.path(), &reborn_home)
        .args(["run", "-m", "ping"])
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
fn run_accepts_configured_cli_tenant_and_agent_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[identity]
tenant = "reborn-cli"
default_agent = "reborn-cli-agent"
default_owner = "operator"
"#,
    )
    .expect("write config");

    let output = isolated_no_llm_command(&workspace, &reborn_home)
        .args(["run", "-m", "ping"])
        .output()
        .expect("ironclaw-reborn run should not crash");
    assert!(
        !output.status.success(),
        "run should still fail without a model gateway"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("reborn run did not produce an assistant reply"),
        "stderr should reach normal runtime failure; got: {stderr}"
    );
    assert!(
        !stderr.contains("tenant") && !stderr.contains("default_agent"),
        "tenant/default_agent should be accepted by CLI identity wiring; got: {stderr}"
    );
}

#[test]
fn run_rejects_unsupported_identity_project_scope_field() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("mkdir");
    std::fs::write(
        reborn_home.join("config.toml"),
        r#"
[identity]
tenant = "reborn-cli"
default_agent = "reborn-cli-agent"
default_owner = "operator"
default_project = "project-alpha"
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
        "unsupported project scope must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[identity]")
            && stderr.contains("default_project")
            && stderr.contains("not wired"),
        "stderr should explain unsupported project scope; got: {stderr}"
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

#[cfg(feature = "root-llm-provider")]
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

#[cfg(feature = "root-llm-provider")]
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

#[cfg(feature = "root-llm-provider")]
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

fn local_yolo_command(temp: &tempfile::TempDir, args: &[&str]) -> Command {
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&reborn_home).expect("reborn home");
    std::fs::create_dir_all(&home).expect("home");
    let mut command = reborn_command();
    command
        .args(args)
        .env("IRONCLAW_REBORN_HOME", reborn_home)
        .env("IRONCLAW_REBORN_PROFILE", "local-dev-yolo")
        .env("HOME", home);
    command
}
