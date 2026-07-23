use std::path::PathBuf;
#[cfg(unix)]
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn runtime_dockerfile() -> String {
    let repo_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .expect("repo root should be discoverable");
    let path = repo_root.join("Dockerfile");
    std::fs::read_to_string(path).expect("Dockerfile should be readable")
}

fn repo_file(relative: &str) -> PathBuf {
    let repo_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .expect("repo root should be discoverable");
    repo_root.join(relative)
}

fn read_repo_file(relative: &str) -> String {
    std::fs::read_to_string(repo_file(relative)).expect("repo file should be readable")
}

#[cfg(unix)]
struct FakeEntrypoint {
    _temp: tempfile::TempDir,
    bin_dir: PathBuf,
    home_dir: PathBuf,
    default_config: String,
    args_file: PathBuf,
}

#[cfg(unix)]
impl FakeEntrypoint {
    fn path_env(&self) -> String {
        format!("{}:/usr/bin:/bin", self.bin_dir.display())
    }
}

#[cfg(unix)]
fn setup_fake_entrypoint() -> FakeEntrypoint {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    let home_dir = temp.path().join("home");
    let args_file = temp.path().join("args.txt");

    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    write_executable(
        &bin_dir.join("ironclaw"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$IRONCLAW_TEST_ARGS_FILE\"\n",
    );
    write_executable(
        &bin_dir.join("cp"),
        "#!/bin/sh\nprintf '%s\\n' 'api_version = \"ironclaw.runtime/v1\"' > \"$2\"\n",
    );
    install_fake_realpath(&bin_dir);

    FakeEntrypoint {
        _temp: temp,
        bin_dir,
        home_dir,
        default_config: "/opt/ironclaw/defaults/config.toml".to_string(),
        args_file,
    }
}

#[cfg(unix)]
fn setup_fake_entrypoint_recording_cp() -> FakeEntrypoint {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    let home_dir = temp.path().join("home");
    let args_file = temp.path().join("args.txt");

    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    write_executable(
        &bin_dir.join("ironclaw"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$IRONCLAW_TEST_ARGS_FILE\"\n",
    );
    write_executable(
        &bin_dir.join("cp"),
        "#!/bin/sh\nprintf '%s\\n[storage]\\n' \"$1\" > \"$2\"\n",
    );
    install_fake_realpath(&bin_dir);

    FakeEntrypoint {
        _temp: temp,
        bin_dir,
        home_dir,
        default_config: "/opt/ironclaw/defaults/config.toml".to_string(),
        args_file,
    }
}

#[cfg(unix)]
fn write_executable(path: &std::path::Path, content: &str) {
    std::fs::write(path, content).expect("write executable");
    let mut permissions = std::fs::metadata(path)
        .expect("executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("executable permissions");
}

#[cfg(unix)]
fn install_fake_realpath(bin_dir: &std::path::Path) {
    write_executable(
        &bin_dir.join("realpath"),
        "#!/bin/sh\n[ \"${1:-}\" != \"-e\" ] || shift\n[ \"${1:-}\" != \"--\" ] || shift\nprintf '%s\\n' \"$1\"\n",
    );
}

#[test]
fn runtime_image_declares_and_prepares_ironclaw_home() {
    let dockerfile = runtime_dockerfile();

    assert!(
        dockerfile.contains("useradd -m -d /home/ironclaw -u 1000 ironclaw"),
        "runtime image must create the ironclaw user with the expected home directory",
    );
    assert!(
        dockerfile.contains("ENV HOME=/home/ironclaw"),
        "runtime image must set HOME to /home/ironclaw for ~/.ironclaw state",
    );
    assert!(
        dockerfile.contains("WORKDIR /workspace"),
        "runtime image must start in the /workspace working directory",
    );
    assert!(
        dockerfile.contains("mkdir -p /data/ironclaw /workspace"),
        "runtime image must pre-create the IronClaw state dir and workspace before dropping privileges",
    );
    assert!(
        dockerfile.contains("chown -R ironclaw:ironclaw /home/ironclaw /data/ironclaw /workspace"),
        "runtime image must hand the home, IronClaw state dir, and workspace to the non-root user",
    );
}

#[test]
fn ironclaw_dockerfile_keeps_bundled_skills_in_build_context() {
    let dockerfile = read_repo_file("Dockerfile");
    let dockerignore = read_repo_file(".dockerignore");

    assert!(
        dockerfile.matches("COPY skills/ skills/").count() >= 2,
        "planner and builder stages must copy bundled IronClaw skills"
    );
    assert!(
        dockerignore.contains("!skills/**/*.md"),
        ".dockerignore must allow bundled SKILL.md and reference markdown files"
    );
    assert!(
        dockerignore.contains("!crates/**/*.md"),
        ".dockerignore must allow crate markdown files embedded at compile time"
    );
}

#[test]
fn ironclaw_dockerfile_uses_feature_matched_cache_and_loopback_default() {
    let dockerfile = read_repo_file("Dockerfile");

    assert!(
        dockerfile.contains(
            "cargo chef cook \\\n    --profile dist \\\n    --package ironclaw \\\n    --recipe-path recipe.json"
        ),
        "cargo chef cook must target the IronClaw CLI package"
    );
    assert!(
        dockerfile.contains("IRONCLAW_SERVE_HOST=127.0.0.1"),
        "image default serve host must stay loopback; Railway should override to 0.0.0.0"
    );
    assert!(
        dockerfile.contains("config.hosted-single-tenant.toml"),
        "image must include the hosted single-tenant seed config"
    );
    assert!(
        dockerfile.contains("config.hosted-single-tenant-volume.toml"),
        "image must include the hosted single-tenant volume seed config"
    );
}

#[test]
fn ironclaw_runtime_image_includes_sql_debug_clients() {
    let dockerfile = read_repo_file("Dockerfile");

    assert!(
        dockerfile.contains("postgresql-client"),
        "runtime image must include psql for Railway hosted Postgres inspection"
    );
    assert!(
        dockerfile.contains("sqlite3"),
        "runtime image must include sqlite3 for volume-backed libSQL/SQLite inspection"
    );
}

#[test]
fn ironclaw_hosted_single_tenant_seed_config_contains_postgres_storage() {
    let config = read_repo_file("docker/ironclaw/config.hosted-single-tenant.toml");

    assert!(
        config.contains("profile = \"hosted-single-tenant\""),
        "hosted seed config must select the hosted profile"
    );
    assert!(
        config.contains("[storage]") && config.contains("backend = \"postgres\""),
        "hosted seed config must include Postgres storage"
    );
    assert!(
        config.contains("pool_max_size = 32"),
        "hosted seed config must size the shared Postgres pool for runtime concurrency"
    );
    assert!(
        config.contains("worker_count = 0"),
        "hosted seed config must not globally throttle turn runners below model/storage capacity"
    );
    assert!(
        !config.contains("[policy]"),
        "hosted seed config must not include production-only [policy]"
    );
}

#[test]
fn ironclaw_hosted_single_tenant_volume_seed_config_uses_volume_storage() {
    let config = read_repo_file("docker/ironclaw/config.hosted-single-tenant-volume.toml");

    assert!(
        config.contains("profile = \"hosted-single-tenant-volume\""),
        "hosted volume seed config must select the hosted volume profile"
    );
    assert!(
        !config.contains("[storage]") && !config.contains("backend = \"postgres\""),
        "hosted volume seed config must not require Postgres storage"
    );
    assert!(
        !config.contains("[policy]"),
        "hosted volume seed config must not include production-only [policy]"
    );
}

#[test]
fn ironclaw_hosted_single_tenant_seed_config_omits_retired_slack_section() {
    let config = read_repo_file("docker/ironclaw/config.hosted-single-tenant.toml");
    let parsed =
        toml::from_str::<toml::Value>(&config).expect("hosted seed config should be valid TOML");
    assert!(
        parsed.get("slack").is_none(),
        "hosted seed config must not recreate the retired specialized [slack] section",
    );
}

#[test]
fn ironclaw_dockerfile_build_is_covered_by_ci() {
    // The IronClaw Dockerfile is now the canonical root `Dockerfile` (it absorbed
    // the former `Dockerfile` under Tier B), so CI builds it as the
    // default context with the `runtime` target rather than via `-f`. Assert it
    // is built by *some* workflow rather than pinning a single file, so future
    // reorganizations don't silently drop coverage.
    let workflows_dir = repo_file(".github/workflows");
    let covered = std::fs::read_dir(&workflows_dir)
        .expect("workflows dir should be readable")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
        .any(|entry| {
            std::fs::read_to_string(entry.path())
                .map(|content| content.contains("docker build --target runtime"))
                .unwrap_or(false)
        });

    assert!(
        covered,
        "some CI workflow must build the canonical IronClaw Dockerfile (`docker build --target runtime … .`)"
    );
}

#[test]
fn ironclaw_deployment_docs_keep_webui_sso_separate_from_product_auth() {
    let docs = read_repo_file("docs/ironclaw/deploy-ironclaw-cli-docker.md");

    assert!(
        docs.contains("https://<railway-domain>/auth/callback/google"),
        "Railway WebUI SSO docs must use the WebUI login callback"
    );
    assert!(
        docs.contains("Product-auth Google credentials are a separate flow"),
        "deployment docs must keep product-auth separate from WebUI login"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_copies_config_and_builds_default_serve_args() {
    let fake = setup_fake_entrypoint();
    let ignored_legacy_home = fake.home_dir.with_extension("legacy");
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_HOME", &ignored_legacy_home)
        .env("IRONCLAW_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_SERVE_HOST", "0.0.0.0")
        .env("IRONCLAW_REBORN_SERVE_HOST", "127.0.0.1")
        .env("PORT", "4321")
        .env("IRONCLAW_CONFIRM_HOST_ACCESS", "true")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fake.home_dir.join("config.toml")).expect("copied config"),
        "api_version = \"ironclaw.runtime/v1\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(&fake.args_file).expect("captured args"),
        "serve\n--host\n0.0.0.0\n--port\n4321\n--confirm-host-access\n"
    );
    assert!(
        !ignored_legacy_home.exists(),
        "neutral deployment variables must win over legacy aliases"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_accepts_legacy_deployment_variable_aliases() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_REBORN_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_REBORN_PROFILE", "local-dev")
        .env("IRONCLAW_REBORN_SERVE_HOST", "0.0.0.0")
        .env("IRONCLAW_REBORN_SERVE_PORT", "4321")
        .env("IRONCLAW_REBORN_CONFIRM_HOST_ACCESS", "true")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fake.home_dir.join("config.toml")).expect("copied config"),
        "api_version = \"ironclaw.runtime/v1\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(&fake.args_file).expect("captured args"),
        "serve\n--host\n0.0.0.0\n--port\n4321\n--confirm-host-access\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_adopts_existing_legacy_railway_home() {
    let fake = setup_fake_entrypoint();
    let volume_root = fake._temp.path().join("volume");
    let legacy_home = volume_root.join("ironclaw-reborn");
    std::fs::create_dir_all(&legacy_home).expect("legacy home dir");
    std::fs::write(
        legacy_home.join("config.toml"),
        "api_version = \"legacy.volume/v1\"\n",
    )
    .expect("legacy config");

    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .args(["serve", "--help"])
        .env_clear()
        .env("PATH", fake.path_env())
        .env("RAILWAY_ENVIRONMENT", "production")
        .env("RAILWAY_VOLUME_MOUNT_PATH", &volume_root)
        .env("IRONCLAW_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(legacy_home.join("config.toml")).expect("legacy config"),
        "api_version = \"legacy.volume/v1\"\n"
    );
    assert!(
        !volume_root.join("ironclaw").exists(),
        "the canonical home must not shadow an existing legacy volume"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_binds_all_interfaces_on_railway_without_explicit_host() {
    // Regression: with no explicit IRONCLAW_SERVE_HOST, a Railway
    // deployment (detected via RAILWAY_* markers) must bind 0.0.0.0 so the
    // platform health check / ingress can reach the container. A loopback bind
    // fails the deploy — the class the checked-in Railway config previously hit.
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_DEFAULT_CONFIG", &fake.default_config)
        .env("RAILWAY_ENVIRONMENT", "production")
        .env("IRONCLAW_ALLOW_EPHEMERAL_RAILWAY", "true")
        .env("PORT", "8080")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&fake.args_file).expect("captured args"),
        "serve\n--host\n0.0.0.0\n--port\n8080\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_keeps_loopback_default_off_railway() {
    // Off-Railway (no RAILWAY_* markers, no explicit host) the conservative
    // loopback default is preserved so a local `docker run` does not bind
    // publicly by surprise.
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_DEFAULT_CONFIG", &fake.default_config)
        .env("PORT", "8080")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&fake.args_file).expect("captured args"),
        "serve\n--host\n127.0.0.1\n--port\n8080\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_selects_hosted_single_tenant_seed_config() {
    let fake = setup_fake_entrypoint_recording_cp();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_PROFILE", "hosted-single-tenant")
        .env("IRONCLAW_ALLOW_EPHEMERAL_RAILWAY", "true")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fake.home_dir.join("config.toml")).expect("copied config"),
        "/opt/ironclaw/defaults/config.hosted-single-tenant.toml\n[storage]\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_selects_hosted_single_tenant_volume_seed_config() {
    let fake = setup_fake_entrypoint_recording_cp();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_PROFILE", "hosted-single-tenant-volume")
        .env("IRONCLAW_ALLOW_EPHEMERAL_RAILWAY", "true")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fake.home_dir.join("config.toml")).expect("copied config"),
        "/opt/ironclaw/defaults/config.hosted-single-tenant-volume.toml\n[storage]\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_passes_explicit_args_through() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .args(["serve", "--help"])
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&fake.args_file).expect("captured args"),
        "serve\n--help\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_resolves_known_env_placeholders_in_explicit_args() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .args(["serve", "--host", "$IRONCLAW_SERVE_HOST", "--port", "$PORT"])
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_SERVE_HOST", "0.0.0.0")
        .env("PORT", "4321")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&fake.args_file).expect("captured args"),
        "serve\n--host\n0.0.0.0\n--port\n4321\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_resolves_legacy_env_placeholders_in_explicit_args() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .args([
            "serve",
            "--host",
            "$IRONCLAW_REBORN_SERVE_HOST",
            "--port",
            "${IRONCLAW_REBORN_SERVE_PORT}",
        ])
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_REBORN_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_REBORN_SERVE_HOST", "0.0.0.0")
        .env("IRONCLAW_REBORN_SERVE_PORT", "4321")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&fake.args_file).expect("captured args"),
        "serve\n--host\n0.0.0.0\n--port\n4321\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_preserves_existing_config() {
    let fake = setup_fake_entrypoint();
    std::fs::create_dir_all(&fake.home_dir).expect("home dir");
    std::fs::write(
        fake.home_dir.join("config.toml"),
        "api_version = \"custom.local/v1\"\n",
    )
    .expect("existing config");

    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .args(["serve", "--help"])
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fake.home_dir.join("config.toml")).expect("preserved config"),
        "api_version = \"custom.local/v1\"\n"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_migrates_disabled_legacy_slack_fields() {
    let fake = setup_fake_entrypoint();
    std::fs::create_dir_all(&fake.home_dir).expect("home dir");
    std::fs::write(
        fake.home_dir.join("config.toml"),
        r#"api_version = "ironclaw.runtime/v1"

[boot]
profile = "hosted-single-tenant-volume"

[slack]
enabled = false
signing_secret_env = "IRONCLAW_SLACK_SIGNING_SECRET"
bot_token_env = "IRONCLAW_SLACK_BOT_TOKEN"
"#,
    )
    .expect("existing config");

    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_PROFILE", "hosted-single-tenant-volume")
        .env("IRONCLAW_ALLOW_EPHEMERAL_RAILWAY", "true")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config =
        std::fs::read_to_string(fake.home_dir.join("config.toml")).expect("migrated config");
    assert!(config.contains("[slack]"));
    assert!(config.contains("enabled = false"));
    assert!(!config.contains("signing_secret_env"));
    assert!(!config.contains("bot_token_env"));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Removed disabled legacy Slack setup fields")
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_rejects_default_config_outside_opt_ironclaw() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_DEFAULT_CONFIG", "/etc/passwd")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(!output.status.success(), "entrypoint should reject path");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_DEFAULT_CONFIG must be under /opt/ironclaw"),
        "stderr: {stderr}"
    );
}

#[test]
#[cfg(unix)]
fn ironclaw_entrypoint_rejects_default_config_that_resolves_outside_opt_ironclaw() {
    let fake = setup_fake_entrypoint();
    write_executable(
        &fake.bin_dir.join("realpath"),
        "#!/bin/sh\nprintf '%s\\n' '/etc/passwd'\n",
    );
    let output = Command::new("sh")
        .arg(repo_file("docker/ironclaw/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_HOME", &fake.home_dir)
        .env("IRONCLAW_DEFAULT_CONFIG", "/opt/ironclaw/config.toml")
        .env("IRONCLAW_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(!output.status.success(), "entrypoint should reject path");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_DEFAULT_CONFIG must resolve under /opt/ironclaw"),
        "stderr: {stderr}"
    );
}
