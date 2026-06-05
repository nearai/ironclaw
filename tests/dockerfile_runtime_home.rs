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
}

#[test]
fn reborn_dockerfile_build_is_covered_by_ci() {
    let workflow = read_repo_file(".github/workflows/test.yml");

    assert!(
        workflow.contains("docker build -f Dockerfile.reborn"),
        "CI docker-build job must build the Reborn CLI Dockerfile"
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
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    let home_dir = temp.path().join("home");
    let default_config = temp.path().join("default.toml");
    let args_file = temp.path().join("args.txt");
    let fake_bin = bin_dir.join("ironclaw-reborn");

    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    std::fs::write(&default_config, "api_version = \"ironclaw.runtime/v1\"\n")
        .expect("default config");
    std::fs::write(
        &fake_bin,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$IRONCLAW_REBORN_TEST_ARGS_FILE\"\n",
    )
    .expect("fake binary");
    let mut permissions = std::fs::metadata(&fake_bin)
        .expect("fake binary metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_bin, permissions).expect("fake binary permissions");

    let path = format!("{}:/usr/bin:/bin", bin_dir.display());
    let output = Command::new("sh")
        .arg(repo_file("docker/reborn/entrypoint.sh"))
        .env_clear()
        .env("PATH", &path)
        .env("IRONCLAW_REBORN_HOME", &home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", &default_config)
        .env("IRONCLAW_REBORN_SERVE_HOST", "0.0.0.0")
        .env("PORT", "4321")
        .env("IRONCLAW_REBORN_CONFIRM_HOST_ACCESS", "true")
        .env("IRONCLAW_REBORN_TEST_ARGS_FILE", &args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(home_dir.join("config.toml")).expect("copied config"),
        "api_version = \"ironclaw.runtime/v1\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(&args_file).expect("captured args"),
        "serve\n--host\n0.0.0.0\n--port\n4321\n--confirm-host-access\n"
    );
}

#[test]
#[cfg(unix)]
fn reborn_entrypoint_passes_explicit_args_through() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    let home_dir = temp.path().join("home");
    let default_config = temp.path().join("default.toml");
    let args_file = temp.path().join("args.txt");
    let fake_bin = bin_dir.join("ironclaw-reborn");

    std::fs::create_dir_all(&bin_dir).expect("bin dir");
    std::fs::write(&default_config, "api_version = \"ironclaw.runtime/v1\"\n")
        .expect("default config");
    std::fs::write(
        &fake_bin,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$IRONCLAW_REBORN_TEST_ARGS_FILE\"\n",
    )
    .expect("fake binary");
    let mut permissions = std::fs::metadata(&fake_bin)
        .expect("fake binary metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fake_bin, permissions).expect("fake binary permissions");

    let path = format!("{}:/usr/bin:/bin", bin_dir.display());
    let output = Command::new("sh")
        .arg(repo_file("docker/reborn/entrypoint.sh"))
        .args(["serve", "--help"])
        .env_clear()
        .env("PATH", &path)
        .env("IRONCLAW_REBORN_HOME", &home_dir)
        .env("IRONCLAW_REBORN_DEFAULT_CONFIG", &default_config)
        .env("IRONCLAW_REBORN_TEST_ARGS_FILE", &args_file)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(&args_file).expect("captured args"),
        "serve\n--help\n"
    );
}
