use std::path::PathBuf;
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
        &bin_dir.join("ironclaw-reborn"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$IRONCLAW_REBORN_TEST_ARGS_FILE\"\n",
    );
    write_executable(
        &bin_dir.join("cp"),
        "#!/bin/sh\nprintf '%s\\n' 'api_version = \"ironclaw.runtime/v1\"' > \"$2\"\n",
    );

    FakeEntrypoint {
        _temp: temp,
        bin_dir,
        home_dir,
        default_config: "/opt/ironclaw/reborn/config.toml".to_string(),
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
        &bin_dir.join("ironclaw-reborn"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$IRONCLAW_REBORN_TEST_ARGS_FILE\"\n",
    );
    write_executable(
        &bin_dir.join("cp"),
        "#!/bin/sh\nprintf '%s\\n[storage]\\n' \"$1\" > \"$2\"\n",
    );

    FakeEntrypoint {
        _temp: temp,
        bin_dir,
        home_dir,
        default_config: "/opt/ironclaw/reborn/config.toml".to_string(),
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
        dockerfile.contains("WORKDIR /home/ironclaw"),
        "runtime image must start in the ironclaw home directory",
    );
    assert!(
        dockerfile.contains("mkdir -p /home/ironclaw/.ironclaw"),
        "runtime image must pre-create ~/.ironclaw before dropping privileges",
    );
}

#[test]
fn reborn_dockerfile_keeps_bundled_skills_in_build_context() {
    let dockerfile = read_repo_file("Dockerfile.reborn");
    let dockerignore = read_repo_file(".dockerignore");

    assert!(
        dockerfile.matches("COPY skills/ skills/").count() >= 2,
        "planner and builder stages must copy bundled Reborn skills"
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
fn reborn_dockerfile_uses_feature_matched_cache_and_loopback_default() {
    let dockerfile = read_repo_file("Dockerfile.reborn");

    assert!(
        dockerfile.contains(
            "cargo chef cook \\\n    --profile dist \\\n    --package ironclaw_reborn_cli \\\n    --features webui-v2-beta,slack-v2-host-beta"
        ),
        "cargo chef cook must target the Reborn CLI package with the same features as the final build"
    );
    assert!(
        dockerfile.contains("IRONCLAW_REBORN_SERVE_HOST=127.0.0.1"),
        "image default serve host must stay loopback; Railway should override to 0.0.0.0"
    );
    assert!(
        dockerfile.contains("config.hosted-single-tenant.toml"),
        "image must include the hosted single-tenant seed config"
    );
}

#[test]
fn reborn_hosted_single_tenant_seed_config_contains_postgres_storage() {
    let config = read_repo_file("docker/reborn/config.hosted-single-tenant.toml");

    assert!(
        config.contains("profile = \"hosted-single-tenant\""),
        "hosted seed config must select the hosted profile"
    );
    assert!(
        config.contains("[storage]") && config.contains("backend = \"postgres\""),
        "hosted seed config must include Postgres storage"
    );
    assert!(
        config.contains("pool_max_size = 10"),
        "hosted seed config must size the shared Postgres pool for runtime concurrency"
    );
    assert!(
        !config.contains("[policy]"),
        "hosted seed config must not include production-only [policy]"
    );
}

#[test]
fn reborn_dockerfile_build_is_covered_by_ci() {
    // The Reborn Dockerfile build can live in any CI workflow — it moved from
    // test.yml to platform-and-compat.yml when the cross-cutting jobs were
    // extracted. Assert it is built by *some* workflow rather than pinning a
    // single file, so future reorganizations don't silently drop coverage.
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
                .map(|content| content.contains("docker build -f Dockerfile.reborn"))
                .unwrap_or(false)
        });

    assert!(
        covered,
        "some CI workflow must build the Reborn CLI Dockerfile (`docker build -f Dockerfile.reborn`)"
    );
}

#[test]
fn reborn_deployment_docs_keep_webui_sso_separate_from_product_auth() {
    let docs = read_repo_file("docs/reborn/deploy-reborn-cli-docker.md");

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
fn reborn_entrypoint_copies_config_and_builds_default_serve_args() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/reborn/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_REBORN_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_REBORN_SERVE_HOST", "0.0.0.0")
        .env("PORT", "4321")
        .env("IRONCLAW_REBORN_CONFIRM_HOST_ACCESS", "true")
        .env("IRONCLAW_REBORN_TEST_ARGS_FILE", &fake.args_file)
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
fn reborn_entrypoint_selects_hosted_single_tenant_seed_config() {
    let fake = setup_fake_entrypoint_recording_cp();
    let output = Command::new("sh")
        .arg(repo_file("docker/reborn/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_REBORN_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_PROFILE", "hosted-single-tenant")
        .env("IRONCLAW_REBORN_ALLOW_EPHEMERAL_RAILWAY", "true")
        .env("IRONCLAW_REBORN_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(fake.home_dir.join("config.toml")).expect("copied config"),
        "/opt/ironclaw/reborn/config.hosted-single-tenant.toml\n[storage]\n"
    );
}

#[test]
#[cfg(unix)]
fn reborn_entrypoint_passes_explicit_args_through() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/reborn/entrypoint.sh"))
        .args(["serve", "--help"])
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_REBORN_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_REBORN_TEST_ARGS_FILE", &fake.args_file)
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
fn reborn_entrypoint_resolves_known_env_placeholders_in_explicit_args() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/reborn/entrypoint.sh"))
        .args([
            "serve",
            "--host",
            "$IRONCLAW_REBORN_SERVE_HOST",
            "--port",
            "$PORT",
        ])
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_REBORN_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_REBORN_SERVE_HOST", "0.0.0.0")
        .env("PORT", "4321")
        .env("IRONCLAW_REBORN_TEST_ARGS_FILE", &fake.args_file)
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
fn reborn_entrypoint_preserves_existing_config() {
    let fake = setup_fake_entrypoint();
    std::fs::create_dir_all(&fake.home_dir).expect("home dir");
    std::fs::write(
        fake.home_dir.join("config.toml"),
        "api_version = \"custom.local/v1\"\n",
    )
    .expect("existing config");

    let output = Command::new("sh")
        .arg(repo_file("docker/reborn/entrypoint.sh"))
        .args(["serve", "--help"])
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_REBORN_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", &fake.default_config)
        .env("IRONCLAW_REBORN_TEST_ARGS_FILE", &fake.args_file)
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
fn reborn_entrypoint_rejects_default_config_outside_opt_ironclaw() {
    let fake = setup_fake_entrypoint();
    let output = Command::new("sh")
        .arg(repo_file("docker/reborn/entrypoint.sh"))
        .env_clear()
        .env("PATH", fake.path_env())
        .env("IRONCLAW_REBORN_HOME", &fake.home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", "/etc/passwd")
        .env("IRONCLAW_REBORN_TEST_ARGS_FILE", &fake.args_file)
        .output()
        .expect("entrypoint should run");

    assert!(!output.status.success(), "entrypoint should reject path");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("IRONCLAW_REBORN_DEFAULT_CONFIG must be under /opt/ironclaw"),
        "stderr: {stderr}"
    );
}
