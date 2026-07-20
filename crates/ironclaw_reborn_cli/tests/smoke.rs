// arch-exempt: large_file, centralized CLI and Dockerfile smoke contracts, plan #6058
use std::io::BufRead;
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const INVALID_PROFILE_MESSAGE: &str = "IRONCLAW_REBORN_PROFILE must be one of";

fn reborn_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ironclaw")
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
    let bin = bin_dir.join("ironclaw");
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
fn dockerfile_reborn_builds_with_production_features() {
    let dockerfile = std::fs::read_to_string(workspace_root().join("Dockerfile.reborn"))
        .expect("Dockerfile.reborn");

    assert!(
        dockerfile.matches("libsql,postgres").count() >= 2,
        "Dockerfile.reborn must compile both cargo-chef deps and final binary with libsql and postgres: {dockerfile}"
    );
    assert!(
        dockerfile.contains("--bin ironclaw")
            && dockerfile
                .contains("COPY --from=builder /app/target/dist/ironclaw /usr/local/bin/ironclaw"),
        "Dockerfile.reborn must build and copy the canonical ironclaw binary: {dockerfile}"
    );
    assert!(
        dockerfile.contains("corepack enable pnpm")
            && dockerfile.matches("pnpm install --frozen-lockfile").count() >= 2
            && dockerfile.contains("crates/ironclaw_webui/frontend"),
        "Dockerfile.reborn must install WebUI frontend dependencies before cargo-chef and the final binary build: {dockerfile}"
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
fn release_ci_compiles_reborn_for_all_supported_targets() {
    // This is a structural contract for release workflow wiring. Hosted Actions
    // runs provide the behavioral cross-platform compile and startup validation.
    let root = workspace_root();
    let compile_workflow =
        std::fs::read_to_string(root.join(".github/workflows/reborn-release-compile.yml"))
            .expect("Reborn release compile workflow")
            .replace("\r\n", "\n");
    let release_workflow =
        std::fs::read_to_string(root.join(".github/workflows/ironclaw-release.yml"))
            .expect("release workflow")
            .replace("\r\n", "\n");
    let workspace_manifest = std::fs::read_to_string(root.join("Cargo.toml"))
        .expect("workspace manifest")
        .replace("\r\n", "\n");
    let cli_manifest = std::fs::read_to_string(root.join("crates/ironclaw_reborn_cli/Cargo.toml"))
        .expect("Reborn CLI manifest")
        .replace("\r\n", "\n");
    let dist_build_setup = std::fs::read_to_string(root.join(".github/dist-build-setup.yml"))
        .expect("cargo-dist build setup")
        .replace("\r\n", "\n");

    let target_runners = [
        ("x86_64-unknown-linux-gnu", "ubuntu-22.04"),
        ("x86_64-unknown-linux-musl", "ubuntu-22.04"),
        ("aarch64-unknown-linux-gnu", "ubuntu-24.04-arm"),
        ("aarch64-unknown-linux-musl", "ubuntu-24.04-arm"),
        ("x86_64-apple-darwin", "macos-15-intel"),
        ("aarch64-apple-darwin", "macos-15"),
        ("x86_64-pc-windows-msvc", "windows-2022"),
    ];
    let release_features = "libsql,postgres";

    assert_eq!(
        compile_workflow.matches("          - target: ").count(),
        target_runners.len(),
        "Reborn release compile matrix must contain exactly seven targets"
    );
    for (target, runner) in target_runners {
        let matrix_entry = format!("          - target: {target}\n            runner: {runner}\n");
        assert!(
            compile_workflow.contains(&matrix_entry),
            "Reborn release compile matrix must map {target} to {runner}"
        );
    }

    assert!(
        compile_workflow.contains("fail-fast: false")
            && compile_workflow.contains("cargo build --locked --profile dist")
            && compile_workflow.contains("--package ironclaw")
            && compile_workflow.contains("            --bin ironclaw \\\n")
            && !compile_workflow.contains("--bin ironclaw-reborn")
            && compile_workflow.contains("--target \"$TARGET\"")
            && compile_workflow
                .contains(&format!("  REBORN_RELEASE_FEATURES: {release_features}\n"))
            && compile_workflow.contains("            --features \"$REBORN_RELEASE_FEATURES\""),
        "Reborn release CI must fully link the shipping binary and keep all target results"
    );
    assert!(
        compile_workflow.matches("musl: true").count() == 2
            && compile_workflow.contains("sudo apt-get install --yes musl-tools binutils file")
            && compile_workflow.contains("CC_x86_64_unknown_linux_musl=musl-gcc")
            && compile_workflow.contains("CC_aarch64_unknown_linux_musl=musl-gcc")
            && !compile_workflow.contains("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER")
            && !compile_workflow.contains("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER")
            && compile_workflow.contains("node-version: \"22\"")
            && compile_workflow.contains("corepack enable pnpm")
            && compile_workflow.contains("binary: ironclaw.exe")
            && !compile_workflow.contains("binary: ironclaw-reborn")
            && compile_workflow.contains("core.longpaths true"),
        "release CI must use musl-gcc for C dependencies without overriding Rust's self-contained musl linker"
    );
    for matrix_entry in [
        concat!(
            "          - target: x86_64-unknown-linux-musl\n",
            "            runner: ubuntu-22.04\n",
            "            binary: ironclaw\n",
            "            musl: true\n",
            "            cc_env: CC_x86_64_unknown_linux_musl=musl-gcc\n",
        ),
        concat!(
            "          - target: aarch64-unknown-linux-musl\n",
            "            runner: ubuntu-24.04-arm\n",
            "            binary: ironclaw\n",
            "            musl: true\n",
            "            cc_env: CC_aarch64_unknown_linux_musl=musl-gcc\n",
        ),
    ] {
        assert!(
            compile_workflow.contains(matrix_entry),
            "musl matrix entry must bind its target to the matching compiler: {matrix_entry}"
        );
    }
    let configure_musl_compiler = concat!(
        "      - name: Configure musl C compiler\n",
        "        if: matrix.musl\n",
        "        shell: bash\n",
        "        env:\n",
        "          CC_ENV: ${{ matrix.cc_env }}\n",
        "        run: |\n",
        "          echo \"$CC_ENV\" >> \"$GITHUB_ENV\"\n",
    );
    assert!(
        compile_workflow.contains(configure_musl_compiler),
        "musl compiler variables must be applied only to musl matrix entries"
    );
    assert!(
        compile_workflow.contains("name: Verify musl portability\n        if: matrix.musl")
            && compile_workflow.contains("readelf --program-headers --wide")
            && compile_workflow.contains("readelf --dynamic --wide")
            && compile_workflow.contains("INTERP")
            && compile_workflow.contains("(NEEDED)")
            && compile_workflow.contains("name: Smoke compiled binary")
            && compile_workflow.contains("\"$binary_path\" --version")
            && compile_workflow.contains("\"$binary_path\" --help > /dev/null")
            && compile_workflow.contains("\"$binary_path\" profile list --json > /dev/null"),
        "release CI must reject non-portable musl binaries and smoke the exact native artifacts"
    );

    let build_position = compile_workflow
        .find("name: Compile ironclaw")
        .expect("compile step");
    let linkage_position = compile_workflow
        .find("name: Verify musl portability")
        .expect("musl linkage step");
    let smoke_position = compile_workflow
        .find("name: Smoke compiled binary")
        .expect("binary smoke step");
    let upload_position = compile_workflow
        .find("name: Upload compile evidence")
        .expect("compile evidence upload step");
    assert!(
        build_position < linkage_position
            && linkage_position < smoke_position
            && smoke_position < upload_position,
        "linkage and runtime validation must gate artifact upload"
    );
    assert!(
        compile_workflow.contains("name: reborn-compile-${{ matrix.target }}")
            && compile_workflow.contains("if-no-files-found: error")
            && !compile_workflow.contains("name: artifacts-reborn"),
        "compile evidence must stay outside cargo-dist's artifacts-* release namespace"
    );
    assert!(
        release_workflow.contains("  push:\n    tags:\n")
            && release_workflow.contains("      - 'ironclaw")
            && compile_workflow.contains("  workflow_call:\n")
            && compile_workflow.contains("  workflow_dispatch:\n")
            && !release_workflow.contains("\n  pull_request:\n")
            && !compile_workflow.contains("  pull_request:\n"),
        "cargo-dist release must be tag-only while the seven-target compile workflow remains an independent manual preflight"
    );
    assert!(
        release_workflow.contains("\n  plan:\n")
            && release_workflow.contains("\n  build-local-artifacts:\n")
            && release_workflow.contains("\n  build-global-artifacts:\n")
            && release_workflow.contains("\n  host:\n")
            && release_workflow.contains("\n  announce:\n")
            && release_workflow.contains("host --steps=create")
            && release_workflow.contains("dist build")
            && release_workflow.contains(
                "dist host ${{ needs.plan.outputs.tag-flag }} --steps=upload --steps=release"
            )
            && release_workflow.contains("gh release create")
            && !release_workflow.contains("uses: ./.github/workflows/reborn-release-compile.yml")
            && compile_workflow.contains("permissions:\n  contents: read"),
        "the generated workflow must own the cargo-dist plan/build/host/announce path without consuming manual compile evidence"
    );
    assert!(
        workspace_manifest.contains("packages = [\"ironclaw\"]")
            && workspace_manifest.contains("github-build-setup = \"../dist-build-setup.yml\"")
            && workspace_manifest.contains("installers = [\"shell\", \"powershell\", \"msi\"]")
            && workspace_manifest.contains("tag-namespace = \"ironclaw\"")
            && workspace_manifest.contains("pr-run-mode = \"skip\""),
        "cargo-dist must select only the canonical Reborn package and generate the supported installers without claiming the unavailable npm package name"
    );
    for target in [
        "aarch64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "aarch64-unknown-linux-musl",
        "x86_64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "x86_64-pc-windows-msvc",
    ] {
        assert!(
            workspace_manifest.contains(&format!("    \"{target}\",")),
            "cargo-dist target list must contain {target}"
        );
    }
    assert!(
        dist_build_setup.contains("actions/setup-node@60edb5dd545a775178f52524783378180af0d1f8")
            && dist_build_setup.contains("node-version: 22")
            && dist_build_setup.contains("corepack enable pnpm")
            && release_workflow.contains("Install Node.js for the embedded WebUI")
            && release_workflow.contains("corepack enable pnpm"),
        "cargo-dist must install the WebUI build prerequisites before compiling artifacts"
    );
    assert!(
        cli_manifest.contains("[package]\nname = \"ironclaw\"\nversion = \"")
            && cli_manifest.contains("[package.metadata.dist]\ndist = true")
            && cli_manifest.contains("features = [\"libsql\", \"postgres\"]")
            && cli_manifest.contains("[package.metadata.wix]")
            && cli_manifest.contains("[[bin]]\nname = \"ironclaw\""),
        "the canonical Reborn package must be cargo-dist enabled with production features and WiX metadata"
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
        .find("CARGO=(cargo run -q -p ironclaw")
        .expect("launcher should run the Reborn CLI");
    assert!(
        frontend_build < cargo_run,
        "scripts/run-reborn-webui.sh must build frontend/dist before cargo compiles the binary: {launcher}"
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

/// The exact `[llm.default]` bytes every shipped Docker profile config used
/// to bake in before this change (`docker/reborn/config.toml`,
/// `config.hosted-single-tenant.toml`,
/// `config.hosted-single-tenant-volume.toml`, `config.production.toml` all
/// carried this identical block) — a persistent Railway volume from before
/// this change still has it verbatim in its own `config.toml`, since the
/// entrypoint only installs a default config when `$config_path` doesn't
/// exist yet.
#[cfg(unix)]
const STALE_BAKED_LLM_DEFAULT_STUB: &str = "[llm.default]\nprovider_id = \"nearai\"\nmodel = \"deepseek-ai/DeepSeek-V4-Flash\"\napi_key_env = \"NEARAI_API_KEY\"\n";

/// RED (entrypoint one-time volume migration): a persistent volume's
/// `config.toml` carrying the EXACT old baked-in `[llm.default]` stub must
/// have that section stripped on boot, with the pre-migration file backed
/// up alongside as `config.toml.pre-llm-migration` — proving an existing
/// Railway deployment converges onto the "no implicit LLM slot" behavior
/// without an operator having to intervene.
#[cfg(unix)]
#[test]
fn docker_reborn_entrypoint_migrates_a_stale_baked_llm_default_stub() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fake_reborn_bin(&bin_dir);
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("reborn home");
    let original_config = format!(
        "api_version = \"ironclaw.runtime/v1\"\n\n[boot]\nprofile = \"local-dev\"\n\n{STALE_BAKED_LLM_DEFAULT_STUB}\n[slack]\nenabled = false\n"
    );
    let config_path = reborn_home.join("config.toml");
    std::fs::write(&config_path, &original_config).expect("write stale config");

    let output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .arg("--help")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Migrated a stale baked-in [llm.default] stub"),
        "entrypoint must report the migration on stderr: {stderr}"
    );

    let migrated_config = std::fs::read_to_string(&config_path).expect("read migrated config");
    assert!(
        !migrated_config.contains("[llm.default]"),
        "the stale [llm.default] section must be stripped: {migrated_config}"
    );
    assert!(
        migrated_config.contains("profile = \"local-dev\"") && migrated_config.contains("[slack]"),
        "unrelated sections must survive the migration untouched: {migrated_config}"
    );

    let backup_path = reborn_home.join("config.toml.pre-llm-migration");
    let backup_config = std::fs::read_to_string(&backup_path).expect("read backup config");
    assert_eq!(
        backup_config, original_config,
        "the backup must be a byte-for-byte copy of the pre-migration file"
    );

    // A second boot (backup already exists) must not clobber the backup
    // with the now-already-migrated file, and must not re-run the
    // migration (nothing left to strip).
    let second_output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .arg("--help")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("entrypoint should run a second time");
    assert!(
        second_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second_output.stderr)
    );
    let second_stderr = String::from_utf8_lossy(&second_output.stderr);
    assert!(
        !second_stderr.contains("Migrated a stale baked-in [llm.default] stub"),
        "a second boot must not re-run the migration: {second_stderr}"
    );
    let backup_after_second_boot =
        std::fs::read_to_string(&backup_path).expect("read backup config after second boot");
    assert_eq!(
        backup_after_second_boot, original_config,
        "the backup must never be overwritten once written"
    );
}

/// Negative case: an operator-modified `[llm.default]` section (here, a
/// deliberately different model) must be left COMPLETELY untouched — the
/// migration only strips an EXACT match of the old baked-in stub, never a
/// section an operator has since edited in any way.
#[cfg(unix)]
#[test]
fn docker_reborn_entrypoint_does_not_migrate_an_operator_modified_llm_default() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bin_dir = temp.path().join("bin");
    fake_reborn_bin(&bin_dir);
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&reborn_home).expect("reborn home");
    let original_config = "api_version = \"ironclaw.runtime/v1\"\n\n[boot]\nprofile = \"local-dev\"\n\n[llm.default]\nprovider_id = \"nearai\"\nmodel = \"an-operator-chosen-model\"\napi_key_env = \"NEARAI_API_KEY\"\n\n[slack]\nenabled = false\n";
    let config_path = reborn_home.join("config.toml");
    std::fs::write(&config_path, original_config).expect("write operator-modified config");

    let output = Command::new("/bin/sh")
        .arg(workspace_root().join("docker/reborn/entrypoint.sh"))
        .arg("--help")
        .env_clear()
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .env("PATH", fake_bin_path(&bin_dir))
        .env("HOME", temp.path().join("home"))
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("entrypoint should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Migrated a stale baked-in [llm.default] stub"),
        "an operator-modified [llm.default] must never be migrated: {stderr}"
    );
    let unchanged_config = std::fs::read_to_string(&config_path).expect("read config");
    assert_eq!(
        unchanged_config, original_config,
        "an operator-modified [llm.default] must be byte-for-byte unchanged"
    );
    assert!(
        !reborn_home.join("config.toml.pre-llm-migration").exists(),
        "no backup should be written when nothing was migrated"
    );
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
        stdout.contains("IronClaw agent runtime"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("Usage: ironclaw"), "stdout: {stdout}");
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
    // `serve` (the HTTP/auth gateway) and `service` (the OS-service
    // installer that runs it) are compiled into every binary.
    assert!(stdout.contains("serve"), "stdout: {stdout}");
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

/// Onboard's OS-service finale (`OnboardCommand::finish_with_service_and_login_link`)
/// only calls `commands::service::install_and_start` when
/// `should_install_service` sees an interactive session — and every child
/// this suite spawns via `Command::output()`/`.spawn()` sees piped, non-tty
/// stdin, so `onboard` can never be driven into that install-attempt branch
/// through a real subprocess here (`onboard_then_serve_boots_in_degraded_mode_with_an_empty_environment`
/// below pins the non-interactive `service: skipped (non-interactive
/// session)` line onboard prints instead). There is no clean "break $HOME"
/// hook that forces onboard itself into the install branch in this
/// environment, so the next best equivalent — same file convention as
/// `onboard_dry_run_propagates_a_webui_token_io_error_without_mutating_home`,
/// which plants a wrong-type filesystem entry at a write target to force a
/// real I/O error — is to drive `service install` directly. That is the
/// exact call `install_and_start` makes on the interactive path
/// (`ServicePlatform::install` writes the same plist/unit file), so this
/// still pins that a blocked service-definition path surfaces as a clean
/// non-zero exit with a readable error, not a panic or a hang.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn service_install_reports_error_when_service_definition_path_is_blocked() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("mkdir home");

    // Plant a plain file where the service-definition directory needs to
    // be created (`~/Library/LaunchAgents/...` on macOS,
    // `~/.config/systemd/user/...` on Linux), so the install's
    // `create_dir_all(parent)` hits a real "not a directory" I/O error
    // instead of writing the plist/unit successfully.
    #[cfg(target_os = "macos")]
    let blocked_parent = home.join("Library");
    #[cfg(target_os = "linux")]
    let blocked_parent = home.join(".config");
    std::fs::write(&blocked_parent, b"not a directory").expect("plant blocking file");

    let output = Command::new(reborn_bin())
        .args(["service", "install"])
        .env_clear()
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", temp.path().join("reborn-home"))
        .output()
        .expect("ironclaw-reborn service install should run");

    assert!(
        !output.status.success(),
        "a blocked service-definition path must fail `service install`, not silently succeed: \
         stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "install failure must be a handled `anyhow` error surfaced on stderr, not a panic: \
         stderr={stderr}"
    );
    assert!(
        !stderr.trim().is_empty(),
        "install failure must surface a readable error, not swallow it: stderr={stderr}"
    );
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
fn channels_list_reports_not_implemented() {
    assert_not_implemented(
        &["channels", "list"],
        "`channels list` is not implemented yet",
    );
    assert_not_implemented(
        &["channels", "list", "--verbose"],
        "`channels list` is not implemented yet",
    );
    assert_not_implemented(
        &["channels", "list", "--json"],
        "`channels list` is not implemented yet",
    );
}

#[test]
fn hooks_list_reports_not_implemented() {
    assert_not_implemented(&["hooks", "list"], "`hooks list` is not implemented yet");
    assert_not_implemented(
        &["hooks", "list", "--verbose"],
        "`hooks list` is not implemented yet",
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
            stderr.contains("ironclaw skills currently supports profile=local-dev"),
            "stderr: {stderr}"
        );
        assert!(
            stderr.contains(&format!("profile={profile}")),
            "stderr: {stderr}"
        );
    }
}

#[test]
fn logs_reports_not_implemented() {
    assert_not_implemented(&["logs"], "`logs` is not implemented yet");
    assert_not_implemented(&["logs", "--verbose"], "`logs` is not implemented yet");
}

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

fn assert_not_implemented(args: &[&str], expected_message: &str) {
    let output = reborn_command()
        .args(args)
        .output()
        .expect("ironclaw-reborn command should run");

    assert!(
        !output.status.success(),
        "`{}` should fail while disabled",
        args.join(" ")
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(expected_message), "stderr: {stderr}");
    assert!(!stderr.contains("panicked"), "stderr: {stderr}");
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
fn config_set_google_client_id_writes_config_toml() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = reborn_command()
        .args([
            "config",
            "set",
            "google.client_id",
            "abc123.apps.googleusercontent.com",
        ])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", temp.path().join("home"))
        .output()
        .expect("ironclaw config set google.client_id should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("google.client_id: saved"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("to apply: ironclaw service restart"),
        "config set must never auto-restart; it must print the explicit apply step: {stdout}"
    );
    assert_eq!(
        stdout.matches("service restart").count(),
        1,
        "the restart instruction must appear exactly once (remediation text plus the \
         apply-step line must not both print it): {stdout}"
    );

    let config = std::fs::read_to_string(reborn_home.join("config.toml")).expect("read config");
    assert!(config.contains("[google]"), "config: {config}");
    assert!(
        config.contains("client_id = \"abc123.apps.googleusercontent.com\""),
        "config: {config}"
    );
}

/// PR-C round-2 fix: `slack_remediation_text` used to embed its own
/// trailing "run `service restart`" sentence on top of `print_apply_step`'s
/// canonical line, double-printing the restart instruction. Pin the
/// exactly-once invariant the same way the google.client_id test above
/// does.
#[test]
fn config_set_slack_enabled_prints_restart_exactly_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = reborn_command()
        .args(["config", "set", "slack.enabled", "true"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", temp.path().join("home"))
        .output()
        .expect("ironclaw config set slack.enabled should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("slack.enabled: saved"), "stdout: {stdout}");
    assert!(
        stdout.contains("to apply: ironclaw service restart"),
        "config set must never auto-restart; it must print the explicit apply step: {stdout}"
    );
    assert_eq!(
        stdout.matches("service restart").count(),
        1,
        "the restart instruction must appear exactly once (remediation text plus the \
         apply-step line must not both print it): {stdout}"
    );
}

/// PR C review fix (item 1): `status` must read the same `[google]`
/// config.toml section `config set google.*` writes, not just env vars —
/// cheapest observable proof is the asymmetric-partial case, since a fully
/// configured or fully unconfigured backend both print no `google_oauth`
/// line at all (see `resolve_google_oauth_degraded`). Setting only
/// `client_id` through `config set` (no env vars, no redirect_uri) must
/// surface as "partially configured (missing redirect_uri)" — which is only
/// possible if `status` actually read the config file this test just wrote.
#[test]
fn config_set_google_client_id_then_status_reports_partial_from_config_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");

    let set_client_id = reborn_command()
        .args([
            "config",
            "set",
            "google.client_id",
            "abc123.apps.googleusercontent.com",
        ])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", &home)
        .output()
        .expect("ironclaw config set google.client_id should run");
    assert!(
        set_client_id.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&set_client_id.stderr)
    );

    let status = reborn_command()
        .args(["status"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", &home)
        .output()
        .expect("ironclaw status should run");
    assert!(
        status.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(
        stdout.contains("google_oauth")
            && stdout.contains("missing google.redirect_uri")
            && stdout.contains("config set google.redirect_uri"),
        "status must reflect the [google] config.toml section config set wrote, not just env \
         vars, and must give the actionable fix command: {stdout}"
    );
}

#[test]
fn config_set_rejects_unknown_key() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");

    let output = reborn_command()
        .args(["config", "set", "nonsense.key", "value"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("HOME", temp.path().join("home"))
        .output()
        .expect("ironclaw config set should run");

    assert!(!output.status.success(), "unknown key must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown config key"), "stderr: {stderr}");
    assert!(
        !reborn_home.join("config.toml").exists(),
        "an unknown key must not seed config.toml"
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
    assert!(stdout.contains("#compdef ironclaw"), "stdout: {stdout}");
    assert!(stdout.contains("_ironclaw"), "stdout: {stdout}");
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
    assert!(stdout.contains("_ironclaw()"), "stdout: {stdout}");
    assert!(stdout.contains("COMPREPLY"), "stdout: {stdout}");
}

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

#[test]
fn serve_boots_without_user_id_env_var() {
    // A unit env with only HOME/PROFILE and no IRONCLAW_REBORN_WEBUI_USER_ID
    // must fall back to [identity].default_owner (or "reborn-cli" when
    // absent) instead of hard-failing before binding a listener.
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        // Token must be >=32 bytes so it clears its own entropy floor before
        // the user-id fallback under test is reached.
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

/// Proves `serve` boots when launched with cwd=<reborn_home>/workspace, the
/// cwd the installed launchd/systemd service now uses.
/// - Regression: unit-content tests only checked `WorkingDirectory` was
///   present, never that serve actually boots from it.
/// - Companion negative test below shows the prior cwd (reborn_home itself)
///   still fails, proving this test discriminates.
#[test]
fn serve_boots_from_the_workspace_subdir_the_installed_service_now_uses_as_cwd() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    let working_directory = reborn_home.join("workspace");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&working_directory).expect("working directory");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let port = unused_local_port();

    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .current_dir(&working_directory)
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "reborn-smoke-test-cwd-workspace-token-0123456789abcdef",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    wait_for_serve_banner(&mut child);

    let _ = child.kill();
    let _ = child.wait();
}

/// Companion regression pin: cwd=reborn_home itself (the crate's first,
/// insufficient fix attempt) still fails, because reborn_home is an
/// ancestor of the default local-dev skill/extension roots and trips
/// composition's `paths_overlap` check. Guards against reverting the
/// installer back to cwd=reborn_home.
#[test]
fn serve_crash_loops_with_skill_root_overlap_when_cwd_is_reborn_home_itself() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    std::fs::create_dir_all(&reborn_home).expect("reborn home");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let port = unused_local_port();

    let output = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .current_dir(&reborn_home)
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env_remove("IRONCLAW_REBORN_PROFILE")
        .env(
            "IRONCLAW_REBORN_WEBUI_TOKEN",
            "reborn-smoke-test-cwd-reborn-home-token-0123456789abcdef",
        )
        .output()
        .expect("ironclaw-reborn serve should run and exit");

    assert!(
        !output.status.success(),
        "serve launched with cwd=reborn_home must fail closed on the skill-root overlap, \
         not silently boot: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("local-dev workspace root must not overlap default skill root"),
        "expected the exact composition overlap error this fix eliminates for \
         <reborn_home>/workspace: stderr={stderr}"
    );
}

#[test]
fn a_real_env_var_beats_the_config_default_end_to_end() {
    // Railway/service-install spine: operator sets IRONCLAW_REBORN_WEBUI_USER_ID
    // explicitly with no [identity].default_owner configured; must still boot
    // after user-id resolution moved into resolve_webui_user_id_raw.
    // [identity] left unset deliberately — a configured default that diverges
    // from env is a separate, already-covered misconfiguration case.
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

#[test]
fn serve_with_env_auth_seeds_reborn_config_before_binding() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                if stderr_text.contains("ironclaw: WebChat v2 listener") {
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
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                if stderr_text.contains("ironclaw: WebChat v2 listener") {
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

#[test]
fn serve_env_slack_enabled_mounts_slack_events_route() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                if stderr_text.contains("ironclaw: WebChat v2 listener") {
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

/// `true` when `config_text` carries a live (uncommented) `provider_id =`
/// line — used to assert a de-seeded `config.toml` has no `[llm.default]`
/// slot. A plain `.contains("provider_id =")` also matches the stub's own
/// commented-out `# provider_id = "nearai"` example line, so this only
/// counts a line whose first non-whitespace character isn't `#`.
fn config_text_has_live_provider_id(config_text: &str) -> bool {
    config_text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("provider_id =") || trimmed.starts_with("provider_id=")
    })
}

/// Guards the bind-close-then-spawn window every `unused_local_port()`
/// caller below goes through: the helper binds a listener to port 0 to get
/// an OS-assigned free port, reads it back, and closes it — then hands
/// that port number to a spawned `serve` child to bind itself. Between the
/// helper's close and the child's own bind, `cargo test`'s parallel test
/// threads can race for the same freed port (one test's helper grabs the
/// port another test's child is about to bind), causing real cross-talk —
/// proven in CI logs, one test observing another test's HTTP response.
/// Every test that spawns a `serve`-mode child on a port from
/// `unused_local_port()` takes this lock before allocating its port and
/// holds it for the rest of the test (dropped at function end), so no two
/// of these tests' allocate-then-spawn windows can overlap. This is a
/// small serialization fix, not a port-reservation framework — do not
/// extend it into a pool or retry-with-backoff mechanism.
static SERVE_PORT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn unused_local_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral local port")
        .local_addr()
        .expect("ephemeral local addr")
        .port()
}

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
            !stderr.contains("ironclaw: WebChat v2 listener"),
            "serve must not bind after invalid WebUI security config; got: {stderr}"
        );
    }
}

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
        !stderr.contains("ironclaw: WebChat v2 listener"),
        "serve must not bind after SSO admission misconfiguration; got: {stderr}"
    );
}

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
        !stderr.contains("ironclaw: WebChat v2 listener"),
        "serve must not bind with a low-entropy session-signing secret; got: {stderr}"
    );
}

/// Send `request` and read the full response: status line, headers, and
/// (best-effort, non-chunked) body. Used by the CLI-token-login tests below,
/// which need `Location`/JSON body content that [`http_status_line`] doesn't
/// capture.
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

#[derive(Debug)]
struct HttpResponse {
    status_line: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name == name)
            .map(|(_, value)| value.as_str())
    }
}

/// With no SSO provider and a file-sourced webui token, `serve` must mount
/// the CLI-printed `/login?token=` route plus `POST /auth/session/exchange`.
/// A valid token redirects into the ticket hand-off, which then resolves to
/// a real session bearer; an invalid token gets a flat 401.
#[test]
fn serve_mounts_cli_login_route_without_sso() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

/// Security: an env-sourced webui bearer token must not get a mounted
/// CLI-token `/login?token=` route — that route puts the bearer in a public
/// URL query string, where an edge/proxy would capture it in access logs.
///
/// Under root-path serving (#6152) an unmounted `/login` isn't a hard 404:
/// the root SPA wildcard (`static_router`) only fails closed for namespaces
/// composition actually reserved (derived from mounted route descriptors —
/// see `webui_serve.rs::static_router_config_from_descriptors`), so a route
/// nobody mounted here falls through to the ordinary SPA shell like any
/// other client-side path. The security property under test is narrower and
/// still holds: the response must be the generic SPA shell, not this
/// route's own handler (a 302/303 redirect carrying a freshly minted
/// session bearer's ticket).
#[test]
fn serve_does_not_mount_cli_login_route_when_token_is_env_sourced() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

    let login_response = http_response(
        port,
        "GET /login?token=irrelevant HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        "cli login probe with env-sourced token",
    );

    let _ = child.kill();
    let _ = child.wait();

    let login_response = login_response.expect("login probe must complete");
    assert!(
        login_response.status_line.contains(" 200 "),
        "an unmounted /login must fall through to the ordinary SPA shell \
         (200), got: {}",
        login_response.status_line
    );
    assert!(
        login_response.header("location").is_none(),
        "the CLI-only /login route's redirect-with-session-ticket handler \
         must not run for an env-sourced token, got Location: {:?}",
        login_response.header("location")
    );
    assert_eq!(
        login_response.header("content-type"),
        Some("text/html; charset=utf-8"),
        "must be the generic SPA shell response, not a route-specific body"
    );
}

/// With an SSO provider configured, `serve` must not also mount the
/// CLI-token-login route's own `/auth/session/exchange` — that would
/// register the path twice. Proven by the CLI-only `/login?token=` route's
/// handler not running (see `serve_does_not_mount_cli_login_route_when_token_is_env_sourced`
/// for why an unmounted `/login` is a 200 SPA-shell fallthrough rather than a
/// 404 under root-path serving, not this route's own redirect) while
/// `/auth/providers` stays up.
#[test]
fn serve_with_sso_does_not_double_mount_session_exchange() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

    let login_response = http_response(
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

    let login_response = login_response.expect("cli login probe must complete");
    let providers_status = providers_status.expect("providers probe must complete");
    assert!(
        login_response.status_line.contains(" 200 ") && login_response.header("location").is_none(),
        "the CLI-only /login route's own redirect-with-session-ticket \
         handler must not run when SSO is configured (its own \
         /auth/session/exchange would collide with the SSO surface's) — \
         expected the ordinary SPA-shell fallthrough, got: {} location={:?}",
        login_response.status_line,
        login_response.header("location")
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

/// Blocks until `child`'s stderr carries the ready banner. Returns
/// everything captured up to and including the banner line, so callers can
/// also assert on pre-banner diagnostics without their own drain thread.
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
                if stderr_text.contains("ironclaw: WebChat v2 listener") {
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

/// Like [`wait_for_serve_banner`], but keeps capturing `child`'s stderr for
/// the rest of the test (returned as a live-updating buffer) instead of
/// dropping the reader once the banner line is seen. The real-turn tests
/// that use this drive several more HTTP round trips after the banner and
/// occasionally hit a `Connection refused` under CPU-contended parallel test
/// runs — the banner line is flushed just before the listener starts
/// accepting, not after, so a loaded box can have a brief gap. Keeping the
/// capture alive (rather than a one-shot channel that's dropped as soon as
/// the caller returns) means a failure can print what `serve` actually did
/// in that window instead of only "connection refused".
fn wait_for_serve_banner_with_capture(
    child: &mut std::process::Child,
    label: &str,
) -> std::sync::Arc<std::sync::Mutex<String>> {
    let stderr_all = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let stderr = child.stderr.take().expect("stderr should be piped");
    let collector = std::sync::Arc::clone(&stderr_all);
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            let mut guard = collector
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.push_str(&line);
            guard.push('\n');
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if stderr_all
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains("ironclaw: WebChat v2 listener")
        {
            return stderr_all;
        }
        if let Some(status) = child.try_wait().expect("serve child status") {
            panic!(
                "{label}: serve exited before binding with {status}; stderr: {}",
                stderr_all
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            );
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "{label}: serve did not reach listener banner; stderr: {}",
                stderr_all
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
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
        stderr.contains("ironclaw: runtime started"),
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
        stderr.contains("ironclaw: runtime started"),
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
        stderr.contains("ironclaw: runtime started"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains("no LLM selection configured"),
        "Codex API-key auth should prevent stub-gateway warning: {stderr}"
    );
}

// Provider/auth validation is always compiled in: the LLM provider is a
// mandatory dependency of the Reborn CLI.
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
        .expect("ironclaw config init should run");
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

#[cfg(not(feature = "libsql"))]
#[test]
fn onboard_reduced_feature_build_reports_llm_provisioning_as_unavailable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let process_home = temp.path().join("process-home");
    let reborn_home = temp.path().join("reborn-home");
    std::fs::create_dir_all(&process_home).expect("create process home");

    let output = reborn_command()
        .arg("onboard")
        .env_clear()
        .env("HOME", &process_home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("IRONCLAW_DISABLE_OS_KEYCHAIN", "1")
        .output()
        .expect("reduced-feature ironclaw onboard should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("llm_credentials: unavailable in this build"),
        "the feature-off outcome must not be reported as non-interactive: {stdout}"
    );
    assert!(
        stdout.contains("--features full") && stdout.contains("rerun `ironclaw onboard`"),
        "reduced-feature onboarding must print feature-specific remediation: {stdout}"
    );
    assert!(
        !stdout.contains(
            "configure LLM credentials: rerun `ironclaw onboard` from an interactive terminal"
        ),
        "feature-disabled provisioning must not blame terminal interactivity: {stdout}"
    );
}

#[test]
fn onboard_is_idempotent_for_the_webui_token_file() {
    // Token doubles as serve's session-signing key: re-running onboard must
    // preserve it, or every signed session and copied env var breaks.
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

/// Full daemon-case journey: headless `onboard` (no LLM env set) followed by
/// `serve` booted with no LLM overrides.
/// - `onboard` must write NO `[llm.default]` slot when nothing is detected
///   in env — config.toml is the single source of truth, seeded only by an
///   explicit act. Non-LLM artifacts (webui-token, marker, login link) are
///   still provisioned, and output teaches how to configure an LLM later.
/// - `serve`'s runtime-LLM resolution is unchanged: no slot + no env means
///   `resolve_reborn_runtime_llm` returns `Ok(None)`, which is not a
///   boot-time hard failure — serve still binds but logs a `warn!`.
/// - Pins both halves: onboard's teaching output/de-seeded config, and
///   serve's warn-but-still-bind behavior.
#[test]
fn onboard_then_serve_boots_in_degraded_mode_with_an_empty_environment() {
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
        "onboard must succeed non-interactively even with no LLM configured; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );
    let onboard_stdout = String::from_utf8_lossy(&onboard_output.stdout);
    assert!(
        onboard_stdout.contains("service: skipped (non-interactive session)"),
        "headless onboarding must not attempt a launchd/systemd install; stdout: {onboard_stdout}"
    );
    assert!(
        onboard_stdout.contains("login_link: http://127.0.0.1:3000/login?token="),
        "onboard must print the CLI-token login link even with no LLM configured; stdout: \
         {onboard_stdout}"
    );
    assert!(
        onboard_stdout.contains("llm_credentials: skipped (non-interactive session)"),
        "onboard must report the LLM step as skipped (nothing detected in env); stdout: \
         {onboard_stdout}"
    );
    assert!(
        onboard_stdout.contains("configure LLM credentials:")
            && onboard_stdout.contains("export a provider's LLM environment variables"),
        "onboard must teach how to configure an LLM afterward; stdout: {onboard_stdout}"
    );
    assert!(
        reborn_home.join("webui-token").exists(),
        "onboard must provision the webui-token file `serve` reads as a fallback"
    );

    let config_path = reborn_home.join("config.toml");
    let config_text = std::fs::read_to_string(&config_path).expect("read seeded config.toml");
    assert!(
        !config_text_has_live_provider_id(&config_text),
        "config.toml is the single source of truth for `[llm.default]`, written only by an \
         explicit act — a fresh headless onboard with nothing detected in env must not seed a \
         provider: {config_text}"
    );

    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let pre_banner_stderr = wait_for_serve_banner(&mut child);
    assert!(
        pre_banner_stderr.contains("no LLM selection configured"),
        "serve must still bind (runtime resolution is unchanged: Ok(None) is not a boot-time \
         hard failure) but warn that runs will fail until an LLM is wired; stderr: \
         {pre_banner_stderr}"
    );

    let _ = child.kill();
    let _ = child.wait();
}

/// Sibling of `onboard_then_serve_boots_in_degraded_mode_with_an_empty_environment`:
/// a headless onboard run WITH a complete `openai`-shape env (API key +
/// model) must silently WRITE `[llm.default]` to config.toml, and a later
/// `serve` booted without `OPENAI_MODEL` set must resolve the PERSISTED
/// model, not a fresh env re-resolution (which would fall back to openai's
/// catalog default).
/// - Uses `openai`, not `nearai`: `nearai`'s `api_key_required = false`
///   would hit the pre-existing idempotency short-circuit that resolves
///   straight from env and never reaches the new write path. `openai`'s
///   idempotency check only looks at the persisted secret store, so a
///   fresh store here reaches the write and proves it happens.
#[test]
fn onboard_with_complete_llm_env_then_serve_boots_from_the_env_seeded_slot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("home dir");
    const ENV_DETECTED_MODEL: &str = "gpt-test-env-detected-model";

    let onboard_output = reborn_command()
        .arg("onboard")
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("OPENAI_API_KEY", "sk-smoke-test-env-detected-openai-key")
        .env("OPENAI_MODEL", ENV_DETECTED_MODEL)
        .output()
        .expect("ironclaw-reborn onboard should run");
    assert!(
        onboard_output.status.success(),
        "onboard must succeed non-interactively; stderr: {}",
        String::from_utf8_lossy(&onboard_output.stderr)
    );
    let onboard_stdout = String::from_utf8_lossy(&onboard_output.stdout);
    assert!(
        onboard_stdout.contains("llm_credentials: configured provider `openai`")
            && onboard_stdout.contains("from environment"),
        "onboard must report the env-detected provider was silently seeded; stdout: \
         {onboard_stdout}"
    );

    let config_path = reborn_home.join("config.toml");
    let config_text = std::fs::read_to_string(&config_path).expect("read env-seeded config.toml");
    assert!(
        config_text.contains("provider_id = \"openai\"")
            && config_text.contains(&format!("model = \"{ENV_DETECTED_MODEL}\"")),
        "headless onboard with a complete openai-shape env must seed the openai slot with the \
         env-detected model: {config_text}"
    );

    // serve, booted without OPENAI_MODEL (only the key), must resolve the
    // PERSISTED model from config.toml, not fall back to openai's catalog
    // default. IRONCLAW_REBORN_LOG scopes the resolved-LLM debug! trace.
    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .env("OPENAI_API_KEY", "sk-smoke-test-env-detected-openai-key")
        .env("IRONCLAW_REBORN_LOG", "info,ironclaw=debug")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    let pre_banner_stderr = strip_ansi(&wait_for_serve_banner(&mut child));
    assert!(
        !pre_banner_stderr.contains("no LLM selection configured"),
        "serve must boot against the env-seeded slot without the degraded-mode warning; \
         stderr: {pre_banner_stderr}"
    );
    assert!(
        pre_banner_stderr.contains(&format!("model={ENV_DETECTED_MODEL}")),
        "serve must resolve the model PERSISTED in the env-seeded slot, not a fresh \
         env-fallback re-resolution (`OPENAI_MODEL` is deliberately unset at `serve` time, so \
         a fresh env-fallback would resolve openai's catalog default `gpt-5-mini` instead); \
         stderr: {pre_banner_stderr}"
    );

    let _ = child.kill();
    let _ = child.wait();
}

/// Full-chain capstone: onboard's printed CLI-token login link must
/// actually work once `serve` is up, and the minted session must authorize
/// a real request against the composed WebChat v2 API — not just bind a
/// listener.
/// - Uses onboard's OWN provisioned token file (not a hand-seeded one) to
///   drive the login → ticket → exchange flow, then goes one step further
///   and uses the exchanged bearer to call the real `RebornServicesApi`,
///   proving the session is mintable AND usable.
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
        .env("NEARAI_MODEL", "deepseek-ai/DeepSeek-V4-Flash")
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

    // Not required for serve to boot (nearai's api_key_required = false) but
    // exercises the stored-key overlay path a real interactive run takes.
    seed_stored_llm_key(&reborn_home, "nearai", "nearai-smoke-test-session");

    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

    // 1. Onboard-provisioned token at /login?token= must redirect into the
    //    ticket hand-off.
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
        location.starts_with("/?login_ticket="),
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

    // 3. The exchanged bearer must authorize a real request against the
    //    production RebornServicesApi, not just be well-formed — catches a
    //    bearer the auth middleware rejects.
    let api_request = format!(
        "GET /api/webchat/v2/threads HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
        bearer.token
    );
    let api_response = http_response(port, &api_request, "authenticated protected-route probe");

    // 4. Authenticating with the webui token = operator/admin, whether via
    //    raw Bearer or this login link — the bearer must also authorize an
    //    operator-gated route, not just an ordinary authenticated one.
    let operator_request = format!(
        "GET /api/webchat/v2/operator/setup HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
        bearer.token
    );
    let operator_response = http_response(port, &operator_request, "operator-gated route probe");

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

    let operator_response = operator_response.expect("operator-gated route probe must complete");
    assert!(
        !operator_response.status_line.contains(" 403 "),
        "the login link's exchanged bearer must authorize an operator-gated \
         request per the USER-DECIDED LAW (webui-token auth = operator), \
         got: {}; body: {}",
        operator_response.status_line,
        operator_response.body
    );
    assert!(
        operator_response.status_line.contains(" 200 "),
        "GET /api/webchat/v2/operator/setup with a valid operator bearer should succeed, got: {}; body: {}",
        operator_response.status_line,
        operator_response.body
    );
}

/// Minimal blocking chat-completions stub standing in for a real LLM
/// provider. Accepts NEAR AI-shaped (`POST /v1/chat/completions`) requests on
/// an OS-assigned local port, captures each request's raw `Authorization`
/// header on `auth_rx`, and answers with a fixed non-streaming completion —
/// enough for the turn loop to finish successfully without ever reaching a
/// real network endpoint. Any other path (e.g. a models-list probe) gets an
/// empty JSON object so an unexpected preflight call doesn't hang the
/// connection.
///
/// Runs on a background `std::thread` (not tokio) because `smoke.rs` tests
/// spawn `ironclaw-reborn` as a real child process and drive it over plain
/// blocking sockets, matching `http_response`'s style above.
fn spawn_chat_completion_stub() -> (String, std::sync::mpsc::Receiver<Option<String>>) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind stub listener");
    let base_url = format!(
        "http://127.0.0.1:{}",
        listener.local_addr().expect("stub local addr").port()
    );
    let (auth_tx, auth_rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .ok();
            let mut reader = std::io::BufReader::new(&mut stream);
            let mut request_line = String::new();
            if std::io::BufRead::read_line(&mut reader, &mut request_line).unwrap_or(0) == 0 {
                continue;
            }
            let mut headers = Vec::new();
            let mut content_length = 0usize;
            let mut auth_header = None;
            loop {
                let mut line = String::new();
                if std::io::BufRead::read_line(&mut reader, &mut line).unwrap_or(0) == 0 {
                    break;
                }
                let trimmed = line.trim_end_matches(['\r', '\n']);
                if trimmed.is_empty() {
                    break;
                }
                if let Some((name, value)) = trimmed.split_once(':') {
                    let name = name.trim().to_ascii_lowercase();
                    let value = value.trim().to_string();
                    if name == "content-length" {
                        content_length = value.parse().unwrap_or(0);
                    }
                    if name == "authorization" {
                        auth_header = Some(value.clone());
                    }
                    headers.push((name, value));
                }
            }
            let mut body = vec![0u8; content_length];
            if content_length > 0 {
                let _ = std::io::Read::read_exact(&mut reader, &mut body);
            }
            let is_chat_completion = request_line.starts_with("POST /v1/chat/completions");
            // Only report auth for the chat-completions request itself — an
            // authenticated non-chat probe (e.g. a models-list preflight)
            // must not be able to satisfy an assertion meant for the chat call.
            if is_chat_completion {
                let _ = auth_tx.send(auth_header);
            }
            // The reborn turn loop always drives the provider through its
            // streaming method when a progress sink is wired (which it is for
            // a real WebUI-driven turn, unlike an in-process `send_user_
            // message` call) — it sends `"stream": true` and expects an SSE
            // body, not a plain JSON one. Detect it and answer accordingly,
            // mirroring `start_nearai_auth_capture_server` in
            // `ironclaw_reborn_composition`'s runtime tests.
            let wants_stream = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.get("stream").and_then(serde_json::Value::as_bool))
                .unwrap_or(false);

            let response = if is_chat_completion && wants_stream {
                let sse_body = concat!(
                    r#"data: {"choices":[{"delta":{"content":"stub reply: stored key reached the live provider"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#,
                    "\n\n",
                    "data: [DONE]\n\n"
                );
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n{sse_body}"
                )
            } else {
                let response_body = if is_chat_completion {
                    serde_json::json!({
                        "id": "chatcmpl-smoke-stub",
                        "choices": [{
                            "message": {
                                "role": "assistant",
                                "content": "stub reply: stored key reached the live provider"
                            },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 1, "completion_tokens": 1 }
                    })
                } else {
                    serde_json::json!({})
                };
                let body_text = response_body.to_string();
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body_text.len(),
                    body_text
                )
            };
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
    });

    (base_url, auth_rx)
}

/// Insert `base_url = "<stub_base_url>"` into the active (uncommented)
/// `[llm.default]` section written by `models set-provider`, which never
/// carries a `base_url` line itself (see the fixture dumped by that
/// command). Targets the one active `api_key_env = "NEARAI_API_KEY"` line
/// (the commented reference blocks further down the file are prefixed with
/// `#` and don't match this exact, unindented text).
fn patch_config_base_url(reborn_home: &Path, base_url: &str) {
    let config_path = reborn_home.join("config.toml");
    let original = std::fs::read_to_string(&config_path).expect("read config.toml to patch");
    let anchor = "api_key_env = \"NEARAI_API_KEY\"\n";
    assert!(
        original.contains(anchor),
        "expected an active `[llm.default]` NEAR AI selection to patch; config: {original}"
    );
    let patched = original.replacen(anchor, &format!("{anchor}base_url = \"{base_url}\"\n"), 1);
    std::fs::write(&config_path, patched).expect("write patched config.toml");
}

/// Drive a real turn through the WebChat v2 HTTP API against a running
/// `serve` process: exchange the onboard-provisioned webui token for a
/// session bearer (mirrors `onboard_login_link_then_bearer_authorizes_a_
/// protected_request`'s steps 1-2), create a thread, send a message, and
/// poll the timeline until the assistant reply lands. Returns the reply
/// text, or `Err` with the last observed timeline body on timeout.
fn drive_real_turn_via_webui(port: u16, webui_token: &str, label: &str) -> Result<String, String> {
    let login = http_response(
        port,
        &format!(
            "GET /login?token={webui_token} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        ),
        "login-link probe",
    )?;
    let location = login
        .header("location")
        .ok_or_else(|| format!("redirect must carry a Location header, got: {login:?}"))?;
    let ticket = location
        .split("login_ticket=")
        .nth(1)
        .ok_or_else(|| format!("redirect must carry a login_ticket, got: {location}"))?
        .to_string();

    let exchange_body = format!(r#"{{"ticket":"{ticket}"}}"#);
    let exchange_request = format!(
        "POST /auth/session/exchange HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{exchange_body}",
        exchange_body.len()
    );
    let exchange = http_response(port, &exchange_request, "session exchange probe")?;
    let exchange_json: serde_json::Value =
        serde_json::from_str(&exchange.body).map_err(|error| format!("exchange body: {error}"))?;
    let bearer = exchange_json["token"]
        .as_str()
        .ok_or_else(|| format!("exchange response missing token: {exchange_json}"))?
        .to_string();

    let create_thread_body = format!(r#"{{"client_action_id":"smoke-create-thread-{label}"}}"#);
    let create_thread_request = format!(
        "POST /api/webchat/v2/threads HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{create_thread_body}",
        create_thread_body.len()
    );
    let created = http_response(port, &create_thread_request, "create thread")?;
    let created_json: serde_json::Value = serde_json::from_str(&created.body)
        .map_err(|error| format!("create-thread body: {error}"))?;
    let thread_id = created_json["thread"]["thread_id"]
        .as_str()
        .ok_or_else(|| format!("create-thread response missing thread_id: {created_json}"))?
        .to_string();

    let message_body =
        format!(r#"{{"content":"hi","client_action_id":"smoke-send-message-{label}"}}"#);
    let send_request = format!(
        "POST /api/webchat/v2/threads/{thread_id}/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{message_body}",
        message_body.len()
    );
    let sent = http_response(port, &send_request, "send message")?;
    let sent_json: serde_json::Value =
        serde_json::from_str(&sent.body).map_err(|error| format!("send-message body: {error}"))?;
    if let Some(outcome) = sent_json["outcome"].as_str()
        && outcome != "submitted"
    {
        return Err(format!("turn was not submitted: {sent_json}"));
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut last_timeline = String::new();
    while std::time::Instant::now() < deadline {
        let timeline_request = format!(
            "GET /api/webchat/v2/threads/{thread_id}/timeline HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {bearer}\r\nConnection: close\r\n\r\n"
        );
        let timeline = http_response(port, &timeline_request, "timeline poll")?;
        last_timeline = timeline.body.clone();
        if let Ok(timeline_json) = serde_json::from_str::<serde_json::Value>(&timeline.body)
            && let Some(messages) = timeline_json["messages"].as_array()
        {
            for message in messages {
                let kind = message["kind"].as_str().unwrap_or_default();
                let status = message["status"].as_str().unwrap_or_default();
                if kind == "assistant" && status == "finalized" {
                    return Ok(message["content"].as_str().unwrap_or_default().to_string());
                }
                if kind == "assistant" && status == "interrupted" {
                    return Err(format!(
                        "turn failed (assistant message interrupted): {timeline_json}"
                    ));
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(format!(
        "timed out waiting for the assistant reply; last timeline: {last_timeline}"
    ))
}

/// Journey-critical fix (PR #6174) regression pin: a provider selected
/// through `config.toml` (mirrors `models set-provider` / onboard) with its
/// key living ONLY in the encrypted secret store (never an env var) must
/// reach the turn-serving provider for a REAL turn driven through the same
/// WebUI HTTP API the browser uses — not just a boot-time trace.
///
/// Before the fix, cold boot resolved the LLM config, then a separate
/// `apply_startup_stored_llm_key` mechanism was supposed to overlay the
/// stored key onto the model gateway directly — but demonstrably never
/// reached the turn-serving provider (see the runtime.rs fix). This test
/// pins the fix: the stub HTTP server captures the `Authorization` header
/// the live provider actually sends, and asserts it carries the stored key.
#[test]
fn stored_key_reaches_real_turn_via_webui_api() {
    const STORED_KEY: &str = "sk-smoke-real-turn-stored-nearai-key";

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
    let webui_token = std::fs::read_to_string(reborn_home.join("webui-token"))
        .expect("read onboard-provisioned webui-token")
        .trim()
        .to_string();
    assert!(
        !webui_token.is_empty(),
        "onboard-provisioned webui-token must not be empty; stdout: {onboard_stdout}"
    );

    let set_provider_output = reborn_command()
        .args(["models", "set-provider", "nearai", "--model", "test-model"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set-provider should run");
    assert!(
        set_provider_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&set_provider_output.stderr)
    );

    // The stored key lives ONLY in the encrypted secret store — this is the
    // onboard-style credential path (`onboard`'s interactive prompt / the
    // webui settings surface), never an env var.
    seed_stored_llm_key_at_runtime_root(&reborn_home, "nearai", STORED_KEY);

    let (stub_base_url, auth_rx) = spawn_chat_completion_stub();
    patch_config_base_url(&reborn_home, &stub_base_url);

    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        // No NEARAI_API_KEY / NEARAI_SESSION_TOKEN: the stored key is the
        // ONLY thing that can authenticate the live provider.
        .env_remove("NEARAI_API_KEY")
        .env_remove("NEARAI_SESSION_TOKEN")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");
    let stderr_all = wait_for_serve_banner_with_capture(&mut child, "single-boot");

    let turn_result = drive_real_turn_via_webui(port, &webui_token, "single-boot");
    if turn_result.is_err() {
        eprintln!("full stderr:\n{}", stderr_all.lock().unwrap());
    }

    let _ = child.kill();
    let _ = child.wait();

    let reply = turn_result.expect("real turn through the WebUI API must succeed");
    assert!(
        reply.contains("stub reply"),
        "assistant reply should come from the stub provider; got: {reply}"
    );

    let captured_auth = auth_rx
        .recv_timeout(std::time::Duration::from_millis(10))
        .expect("stub must have captured at least one request's Authorization header");
    let captured_auth = captured_auth.expect("captured request must carry an Authorization header");
    assert_eq!(
        captured_auth,
        format!("Bearer {STORED_KEY}"),
        "the live provider must authenticate with the stored key, not a session token or nothing"
    );
}

/// Companion to `stored_key_reaches_real_turn_via_webui_api`: proves the
/// stored-key path is not a one-boot fluke by driving TWO independent `serve`
/// boots (fresh child process each time, same `reborn_home`, no `onboard` or
/// `models set-provider` run again in between) and asserting the second boot
/// also authenticates a real turn with the stored key. This is the cheaper,
/// faithful stand-in for "save settings while serve is running, then
/// restart": both scenarios exercise the same boot-reads-from-disk-and-store
/// path with no env var and no in-process state carried over — driving the
/// live `LlmConfigService` HTTP save route while `serve` is already up would
/// additionally require standing up its multipart auth/session flow, which
/// buys no extra coverage of the fix (the boot-time reload chokepoint is
/// identical either way).
#[test]
fn stored_key_reaches_real_turn_across_fresh_boots() {
    const STORED_KEY: &str = "sk-smoke-restart-stored-nearai-key";

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
    let webui_token = std::fs::read_to_string(reborn_home.join("webui-token"))
        .expect("read onboard-provisioned webui-token")
        .trim()
        .to_string();

    let set_provider_output = reborn_command()
        .args(["models", "set-provider", "nearai", "--model", "test-model"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set-provider should run");
    assert!(
        set_provider_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&set_provider_output.stderr)
    );

    seed_stored_llm_key_at_runtime_root(&reborn_home, "nearai", STORED_KEY);

    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    for boot in 1..=2 {
        let (stub_base_url, auth_rx) = spawn_chat_completion_stub();
        patch_config_base_url_replacing_previous(&reborn_home, &stub_base_url);

        let port = unused_local_port();
        let mut child = reborn_command()
            .args(["serve", "--host", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .env("HOME", &home)
            .env("IRONCLAW_REBORN_HOME", &reborn_home)
            .env_remove("NEARAI_API_KEY")
            .env_remove("NEARAI_SESSION_TOKEN")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| {
                panic!("boot {boot}: ironclaw-reborn serve should start: {error}")
            });
        let stderr_all = wait_for_serve_banner_with_capture(&mut child, &format!("boot {boot}"));

        let turn_result = drive_real_turn_via_webui(port, &webui_token, &format!("boot-{boot}"));
        if turn_result.is_err() {
            eprintln!("boot {boot} full stderr:\n{}", stderr_all.lock().unwrap());
        }

        let _ = child.kill();
        let _ = child.wait();

        turn_result.unwrap_or_else(|error| {
            panic!("boot {boot}: real turn through the WebUI API must succeed: {error}")
        });

        let captured_auth = auth_rx
            .recv_timeout(std::time::Duration::from_millis(10))
            .unwrap_or_else(|_| panic!("boot {boot}: stub must have captured a request"))
            .unwrap_or_else(|| {
                panic!("boot {boot}: captured request missing Authorization header")
            });
        assert_eq!(
            captured_auth,
            format!("Bearer {STORED_KEY}"),
            "boot {boot}: the live provider must authenticate with the stored key on every fresh \
             boot, not just the first"
        );
    }
}

/// Like [`patch_config_base_url`], but replaces whichever `base_url` line is
/// currently in `[llm.default]` — the previous boot's stub port is dead by
/// the time the next boot patches it in, so the second call in
/// `stored_key_reaches_real_turn_across_fresh_boots` must overwrite rather
/// than duplicate the line `patch_config_base_url` already inserted.
fn patch_config_base_url_replacing_previous(reborn_home: &Path, base_url: &str) {
    let config_path = reborn_home.join("config.toml");
    let original = std::fs::read_to_string(&config_path).expect("read config.toml to patch");
    if let Some(start) = original.find("base_url = \"http://127.0.0.1:") {
        let rest = &original[start..];
        let end = rest
            .find('\n')
            .map(|index| start + index + 1)
            .unwrap_or(original.len());
        let mut patched = String::with_capacity(original.len());
        patched.push_str(&original[..start]);
        patched.push_str(&format!("base_url = \"{base_url}\"\n"));
        patched.push_str(&original[end..]);
        std::fs::write(&config_path, patched).expect("write patched config.toml");
    } else {
        patch_config_base_url(reborn_home, base_url);
    }
}

/// Seed the local-dev encrypted secret store with an LLM API key for
/// `provider_id`, through the same `open_local_dev_secret_store` +
/// `LlmKeyStore::put` opener `onboard`'s interactive credential prompt uses
/// — bypassing the prompt UI. Also seeds the cached master-key dotfile first
/// so the resolver never reaches the OS keychain (a headless run would
/// otherwise hang on a GUI keychain prompt — see
/// `onboard_with_complete_llm_env_then_serve_boots_from_the_env_seeded_slot`'s
/// call site for the same rationale).
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

/// Seed the stored LLM key at the SAME secret-store root `serve` actually
/// reads from for a `local-dev` boot — `<reborn_home>/local-dev/…` (see
/// `local_runtime_storage_root` / `RebornProfile::local_runtime_storage_
/// subdir`), NOT the bare `reborn_home` root [`seed_stored_llm_key`] (and
/// `onboard`'s own interactive credential prompt) write to.
///
/// This distinction matters for these real-turn tests specifically: they
/// pin the fix to `RebornLlmReloadAdapter::reload`, which reads through
/// `RebornRuntime`'s own `services.secret_store()` — rooted at the
/// `local-dev` subdirectory. Seeding through the bare-root opener instead
/// (matching `onboard`'s CLI path) would silently miss that store and fail
/// for an unrelated reason (a pre-existing root mismatch between `onboard`'s
/// credential prompt and the runtime's own store, out of scope here — filed
/// as a follow-up). The webui settings-save path this fix's reload
/// mechanism mirrors always writes through `services.secret_store()`
/// directly, so this is the faithful root to seed for these tests.
fn seed_stored_llm_key_at_runtime_root(reborn_home: &Path, provider_id: &str, key: &str) {
    let runtime_root = reborn_home.join("local-dev");
    std::fs::create_dir_all(&runtime_root).expect("runtime local-dev root dir");
    seed_stored_llm_key(&runtime_root, provider_id, key);
}

/// A key stored via `onboard`/`models set-provider` for an
/// `api_key_required = true` provider (openai/anthropic) must reach
/// `serve`'s runtime resolution — `apply_startup_stored_llm_key` must run
/// before `resolve_reborn_runtime_llm` fails closed on `ApiKeyEnvUnset`.
/// `nearai` (`api_key_required = false`) never exercises this path, so the
/// existing daemon-case capstones didn't catch it.
/// - Crate smoke tier (real binary spawn): the bug lives in serve's
///   pre-async-runtime boot sequence, so only a real-process boot proves
///   the fix ordering.
/// - Also proves the model scripted via `models set-provider --model` is
///   the model `serve` actually resolves, not just A model — asserted via
///   the resolved-LLM `debug!` trace, scoped into view with
///   `IRONCLAW_REBORN_LOG` (never `info!`/`warn!` per the REPL/TUI logging
///   rule). Uses a non-default model name to rule out a hardcoded fallback.
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

    // models set-provider is the non-interactive equivalent of onboard's
    // credential prompt; SCRIPTED_MODEL is deliberately non-default.
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

    seed_stored_llm_key_at_runtime_root(&reborn_home, "openai", "sk-smoke-test-stored-openai-key");

    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        // No OPENAI_API_KEY: the stored key must be what makes this boot.
        // Target is `ironclaw` (the bin's normalized crate name, not
        // the `ironclaw_reborn_cli` package this test itself compiles as).
        // `ironclaw_reborn_composition=debug` is also needed to observe
        // `RebornLlmReloadAdapter::reload`'s own `key_applied` trace below —
        // that's the mechanism that actually swaps the placeholder gateway
        // for the stored-key-backed openai provider (PR #6174 item A).
        .env(
            "IRONCLAW_REBORN_LOG",
            "info,ironclaw=debug,ironclaw_reborn_composition=debug",
        )
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
    // Regression pin for PR #6174 item A: `openai` has `api_key_required =
    // true`, so unlike `nearai` it used to hit the STRICT
    // `resolve_reborn_runtime_llm` inside `RebornLlmReloadAdapter::reload`
    // (boot-time and Settings -> Inference save both go through it) and fail
    // closed on `ApiKeyEnvUnset` before ever reaching the stored-key lookup
    // — the placeholder gateway was silently never replaced. This asserts
    // the live reload actually ran and applied the seeded key.
    assert!(
        pre_banner_stderr.contains("LLM reload applied to the live provider")
            && pre_banner_stderr.contains("key_applied=true"),
        "the seeded openai key must be found and applied to the live provider by the boot-time \
         LLM reload, not left on the placeholder gateway; stderr: {pre_banner_stderr}"
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

/// Regression pin: nearai's coded default base URL used to key on API-key
/// presence at resolve time (cloud when a key was already attached, private
/// otherwise). An operator-stored key (attached to the runtime *after*
/// config resolution, via `apply_startup_stored_llm_key`) always missed that
/// window, so a key stored through `onboard`/`models set-provider` — the
/// exact path the webui settings surface uses — left the live runtime
/// pinned to the keyless private endpoint even though the same key made the
/// admin `test_connection` probe and the settings-panel catalog snapshot
/// correctly report cloud. Fixed by making nearai's coded default
/// unconditionally cloud (`ironclaw_llm::resolution::default_nearai_base_url`),
/// so resolution order no longer matters.
///
/// `nearai`'s `api_key_required = false`, so unlike `openai` this boots
/// successfully either way — the only observable symptom was the wrong base
/// URL — asserted here on the resolved-LLM `debug!` trace's `base_url`
/// field, which fires during boot-time config resolution (no stored-key
/// application needed to observe the fix: it holds even in the fully
/// keyless case this test drives).
#[test]
fn onboard_nearai_then_serve_boots_with_cloud_base_url() {
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
        .args(["models", "set-provider", "nearai"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set-provider should run");
    assert!(
        set_provider_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&set_provider_output.stderr)
    );

    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        // No NEARAI_API_KEY, no NEARAI_BASE_URL: nothing pins the base URL —
        // proving the coded default itself is cloud, not a key-presence race.
        .env_remove("NEARAI_API_KEY")
        .env_remove("NEARAI_BASE_URL")
        .env("IRONCLAW_REBORN_LOG", "info,ironclaw=debug")
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
        pre_banner_stderr.contains("provider_id=nearai"),
        "resolved-LLM trace must name the nearai provider; stderr: {pre_banner_stderr}"
    );
    assert!(
        pre_banner_stderr.contains("base_url=https://cloud-api.near.ai"),
        "nearai's coded default must be the cloud endpoint even fully keyless; \
         stderr: {pre_banner_stderr}"
    );
    assert!(
        !pre_banner_stderr.contains("base_url=https://private.near.ai"),
        "resolved-LLM trace must never carry the retired keyless-private default; \
         stderr: {pre_banner_stderr}"
    );
}

/// Companion to `onboard_nearai_then_serve_boots_with_cloud_base_url`: that
/// test only pins the fully-keyless coded default, since `onboard`/`models
/// set-provider` never stores a NearAI key non-interactively — a regression
/// in the boot-time LLM reload (`RebornLlmReloadAdapter::reload`) or its
/// ordering relative to config resolution would still pass it. This test
/// seeds an encrypted NearAI credential AFTER provider selection, AT THE
/// SAME RUNTIME STORAGE ROOT `serve` actually opens
/// (`local_runtime_storage_root`, i.e. `<reborn_home>/local-dev` —
/// `seed_stored_llm_key_at_runtime_root`, not the bare-root
/// `seed_stored_llm_key`, which writes to a root `serve` never reads), with
/// both NearAI env overrides removed. Asserts through the resolved-LLM
/// boot-trace seam that the late-attached stored credential still resolves
/// to the cloud endpoint, AND — the discriminating half, since
/// `default_nearai_base_url` is unconditionally cloud regardless of
/// key-presence — through `RebornLlmReloadAdapter::reload`'s own
/// `key_applied` debug trace that the seeded credential was actually found
/// and applied to the live provider, not silently skipped.
#[test]
fn onboard_nearai_stored_key_then_serve_boots_with_cloud_base_url() {
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
        .args(["models", "set-provider", "nearai"])
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        .output()
        .expect("ironclaw-reborn models set-provider should run");
    assert!(
        set_provider_output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&set_provider_output.stderr)
    );

    seed_stored_llm_key_at_runtime_root(
        &reborn_home,
        "nearai",
        "session-smoke-test-stored-nearai-key",
    );

    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let port = unused_local_port();
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        // No NEARAI_API_KEY, no NEARAI_BASE_URL: only the stored credential
        // (applied by the boot-time LLM reload) is present — proving the
        // late-attached path also lands on cloud.
        .env_remove("NEARAI_API_KEY")
        .env_remove("NEARAI_BASE_URL")
        // `key_applied` is emitted by `ironclaw_reborn_composition`'s
        // `RebornLlmReloadAdapter::reload`, not the `ironclaw_reborn_cli`
        // binary crate — the default filter caps that crate at `info`, so
        // it must be named explicitly.
        .env(
            "IRONCLAW_REBORN_LOG",
            "info,ironclaw=debug,ironclaw_reborn_composition=debug",
        )
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
        pre_banner_stderr.contains("provider_id=nearai"),
        "resolved-LLM trace must name the nearai provider; stderr: {pre_banner_stderr}"
    );
    assert!(
        pre_banner_stderr.contains("base_url=https://cloud-api.near.ai"),
        "a NearAI key stored after provider selection and applied via the boot-time LLM reload \
         must still resolve to the cloud endpoint; stderr: {pre_banner_stderr}"
    );
    assert!(
        !pre_banner_stderr.contains("base_url=https://private.near.ai"),
        "resolved-LLM trace must never carry the retired keyless-private default, even on the \
         late-stored-key path; stderr: {pre_banner_stderr}"
    );
    assert!(
        pre_banner_stderr.contains("LLM reload applied to the live provider")
            && pre_banner_stderr.contains("key_applied=true"),
        "the seeded credential must actually be found and applied to the live provider — a \
         discriminating signal independent of the unconditionally-cloud coded default: \
         stderr: {pre_banner_stderr}"
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
    // Sanity: config carries no [llm.default] slot; this boots purely off
    // NEARAI_API_KEY via the env-fallback path, matching Railway's shape.
    let config_text =
        std::fs::read_to_string(reborn_home.join("config.toml")).expect("read seeded config.toml");
    assert!(
        !config_text_has_live_provider_id(&config_text),
        "config: {config_text}"
    );

    let _serve_port_guard = SERVE_PORT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

    // Spawn + poll try_wait() with a deadline instead of blocking .output():
    // a regression that binds a listener instead of exiting must not hang
    // this test (and CI) forever.
    let mut child = reborn_command()
        .args(["serve", "--host", "127.0.0.1", "--port", "0"])
        .env("HOME", &home)
        .env("IRONCLAW_REBORN_HOME", &reborn_home)
        // No OPENAI_API_KEY set.
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ironclaw-reborn serve should start");

    // Drain stdout/stderr on background threads so a full pipe buffer can't
    // block the child from exiting.
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

/// `status` companion: reprinting the login link must also respect
/// env-token precedence, unless `status` already knows the OS service isn't
/// running, in which case that takes priority — no point advertising a
/// login credential into a `serve` that isn't listening.
/// - The service-state query is host-wide, not scoped to this test's temp
///   $HOME, so the exact `service:` value can't be pinned (CI reads "not
///   installed"; a dev host with the real service may read differently).
///   Assert the invariant instead: `login_link` is always absent, and
///   `login_note` matches whichever branch the observed state took.
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
        // `apply_service_suppression` passes `Running`/`Unknown` through
        // unchanged by design — `Unknown` is what CI runners actually hit
        // (no user dbus session to query), so this arm is load-bearing for
        // CI, not just local-dev symmetry.
        "running" | "unknown" => assert!(
            stdout.contains("login_note:") && stdout.contains("IRONCLAW_REBORN_WEBUI_TOKEN is set"),
            "service is running or unknown, so the env-token note must still win: {stdout}"
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

/// `config.toml`'s content here ("custom config\n") is deliberately not
/// valid TOML — it used to prove Preserve-policy writes are byte-for-byte,
/// unvalidated. That's still true (the Preserve write runs before LLM-
/// credential provisioning ever inspects the file), but
/// `already_configured_outcome` now fails loud on an unparseable
/// `config.toml` rather than silently treating it as "not yet configured"
/// (see that function's doc): a corrupt config is a real problem onboard
/// must surface, not quietly succeed over. So the overall command now fails
/// — this test pins that (a) it fails with a parse-error message, and (b)
/// the artifacts written/preserved BEFORE that failure (config.toml's exact
/// bytes, the marker, providers.json) are still correct on disk, since
/// `write_default_config_files` and the marker/master-key steps all run
/// ahead of the LLM-credential step that fails.
// The pinned failure (the LLM-credential step parsing the malformed
// config.toml) exists only when the provider feature compiles that step in;
// without it onboard legitimately succeeds, so a provider-free build cannot
// assert this behavior.
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
        !output.status.success(),
        "onboard must fail when a pre-existing config.toml can't be parsed, not silently \
         succeed over a broken config; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not parse config file"),
        "stderr must explain the config.toml parse failure: {stderr}"
    );

    let config_text =
        std::fs::read_to_string(reborn_home.join("config.toml")).expect("read config");
    assert_eq!(
        config_text, "custom config\n",
        "the Preserve write runs before the LLM-credential step, so the malformed file on disk \
         must stay untouched even though the overall command later fails"
    );
    let marker_text =
        std::fs::read_to_string(reborn_home.join(".onboard-completed.json")).expect("read marker");
    assert_eq!(marker_text, "custom marker\n");
    assert!(
        reborn_home.join("providers.json").exists(),
        "missing providers file — write_default_config_files runs before the failing step"
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

/// With no cached master-key dotfile and the OS keychain suppressed
/// (`IRONCLAW_DISABLE_OS_KEYCHAIN=1`, this test's real-binary equivalent of
/// a headless Linux / denied prompt), onboard must print the
/// SECRETS_MASTER_KEY/dotfile fallback note and still exit 0 — never fail
/// onboarding just because the keychain step provisioned nothing. A real
/// successful keychain write needs an actual OS keychain and stays
/// manual/E2E only.
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
    // Dotfile creation stays the resolver's own auto-gen-on-first-boot job,
    // not onboarding's.
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
///
/// The dotfile is seeded at the RUNTIME storage root
/// (`<reborn_home>/local-dev/…`, `local_runtime_storage_root`'s subdir for
/// `RebornProfile::LocalDev`) — the same root the real resolver
/// (`resolve_local_dev_secret_master_key_with_env`) reads/writes and
/// `serve` actually boots against — not the bare `reborn_home` root (PR
/// #6174 item D: `provision_master_key` used to check the bare root, so its
/// `exists()` check was always false and it re-attempted keychain
/// provisioning on every rerun).
#[test]
fn onboard_master_key_provisioning_is_a_noop_once_a_dotfile_is_cached() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let runtime_root = reborn_home.join("local-dev");
    std::fs::create_dir_all(&runtime_root).expect("mkdir");
    std::fs::write(
        runtime_root.join(".reborn-local-dev-secrets-master-key"),
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
        std::fs::read_to_string(runtime_root.join(".reborn-local-dev-secrets-master-key"))
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

#[test]
fn release_ci_publishes_reborn_without_enabling_legacy_or_docker_paths() {
    let root = workspace_root();
    let release_workflow =
        std::fs::read_to_string(root.join(".github/workflows/ironclaw-release.yml"))
            .expect("release workflow")
            .replace("\r\n", "\n");
    let docker_workflow = std::fs::read_to_string(root.join(".github/workflows/docker.yml"))
        .expect("Docker workflow")
        .replace("\r\n", "\n");
    let code_style_workflow =
        std::fs::read_to_string(root.join(".github/workflows/code_style.yml"))
            .expect("code style workflow")
            .replace("\r\n", "\n");
    let workspace_manifest = std::fs::read_to_string(root.join("Cargo.toml"))
        .expect("workspace manifest")
        .replace("\r\n", "\n");
    let cli_manifest = std::fs::read_to_string(root.join("crates/ironclaw_reborn_cli/Cargo.toml"))
        .expect("Reborn CLI manifest")
        .replace("\r\n", "\n");
    let wix_manifest =
        std::fs::read_to_string(root.join("crates/ironclaw_reborn_cli/wix/main.wxs"))
            .expect("Reborn WiX manifest")
            .replace("\r\n", "\n");

    let release_job = |job_name: &str| {
        let job_marker = format!("  {job_name}:\n");
        let job_start = release_workflow
            .match_indices(&job_marker)
            .find_map(|(index, _)| {
                (index == 0 || release_workflow.as_bytes()[index - 1] == b'\n')
                    .then_some(index + job_marker.len())
            })
            .unwrap_or_else(|| panic!("release workflow should define the {job_name} job"));
        let jobs_after_marker = &release_workflow[job_start..];
        let job_body = jobs_after_marker
            .lines()
            .take_while(|line| {
                let trimmed = line.trim_start();
                trimmed.is_empty() || line.len() - trimmed.len() > 2
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !job_body.is_empty(),
            "release workflow should define the {job_name} job body"
        );
        job_body
    };

    let plan_job = release_job("plan");
    assert!(
        plan_job.contains("host --steps=create")
            && plan_job.contains("dist ${{")
            && plan_job.contains("|| 'plan'")
            && plan_job.contains("--output-format=json")
            && plan_job.contains("artifacts-plan-dist-manifest")
            && plan_job.contains("cargo-dist-cache"),
        "cargo-dist plan must create the release and publish its build manifest"
    );

    assert!(
        release_workflow.contains("name: Release\npermissions:\n  \"contents\": \"read\"\n")
            && !plan_job.contains("permissions:"),
        "release CI must default every job to read-only repository permissions"
    );

    let local_build_job = release_job("build-local-artifacts");
    assert!(
        local_build_job.contains("fromJson(needs.plan.outputs.val).ci.github.artifacts_matrix")
            && local_build_job.contains("fail-fast: false")
            && local_build_job.contains("Install Node.js for the embedded WebUI")
            && local_build_job.contains("corepack enable pnpm")
            && local_build_job.contains("dist build ${{ needs.plan.outputs.tag-flag }}")
            && local_build_job.contains("artifacts-build-local-${{ join(matrix.targets, '_') }}"),
        "cargo-dist must build and upload every platform artifact from its generated matrix"
    );
    assert!(
        !local_build_job.contains("permissions:") && !local_build_job.contains("GH_TOKEN:"),
        "untrusted local builds must not receive elevated repository permissions or a GitHub token"
    );

    let global_build_job = release_job("build-global-artifacts");
    assert!(
        global_build_job.contains("build-local-artifacts")
            && global_build_job.contains("\"--artifacts=global\"")
            && global_build_job.contains("artifacts-build-global"),
        "cargo-dist must generate checksums and universal installers after local builds"
    );
    assert!(
        !global_build_job.contains("permissions:") && !global_build_job.contains("GH_TOKEN:"),
        "global packaging must not receive elevated repository permissions or a GitHub token"
    );

    let host_job = release_job("host");
    assert!(
        host_job.contains("needs.plan.outputs.publishing == 'true'")
            && host_job.contains("permissions:\n      \"contents\": \"write\"")
            && host_job.contains("GH_TOKEN:")
            && host_job.contains("--steps=upload --steps=release")
            && host_job.contains("ANNOUNCEMENT_TITLE")
            && host_job.contains("ANNOUNCEMENT_BODY")
            && host_job.contains("PRERELEASE_FLAG")
            && host_job.contains("gh release create")
            && host_job.contains("--title \"$ANNOUNCEMENT_TITLE\"")
            && host_job.contains("--notes-file \"$RUNNER_TEMP/notes.txt\"")
            && host_job.contains("artifacts/*"),
        "cargo-dist host must publish generated assets with generated title, notes, and prerelease state"
    );

    let announce_job = release_job("announce");
    assert!(
        announce_job.contains("- plan")
            && announce_job.contains("- host")
            && announce_job.contains("needs.host.result == 'success'")
            && !announce_job.contains("registry")
            && !announce_job.contains("checksum")
            && !announce_job.contains("permissions:")
            && !announce_job.contains("GH_TOKEN:"),
        "cargo-dist announce must only finalize a successful hosted release, not run the legacy registry path"
    );

    for removed_job_name in [
        "reborn-binary-compile",
        "publish-reborn-binaries",
        "build-wasm-extensions",
        "docker-image",
        "update-registry-checksums",
    ] {
        assert!(
            !release_workflow.contains(&format!("\n  {removed_job_name}:\n")),
            "Reborn-only cargo-dist release must not define the old {removed_job_name} job"
        );
    }
    assert!(
        !release_workflow.contains("uses: ./.github/workflows/docker.yml")
            && !release_workflow.contains("ironclaw-legacy")
            && !release_workflow.contains("ironclaw_legacy")
            && !release_workflow.contains("reborn-compile-"),
        "the release workflow must consume only cargo-dist Reborn artifacts"
    );
    assert!(
        docker_workflow.contains("workflow_dispatch:") && docker_workflow.contains("schedule:"),
        "the independent Docker workflow must remain manually and periodically runnable"
    );
    assert!(
        workspace_manifest.contains("name = \"ironclaw_legacy\"")
            && workspace_manifest.contains("[package.metadata.dist]\ndist = false")
            && workspace_manifest.contains("packages = [\"ironclaw\"]")
            && workspace_manifest.contains("allow-dirty = [\"ci\"]")
            && cli_manifest.contains("[package]\nname = \"ironclaw\"\nversion = \"")
            && cli_manifest.contains("[package.metadata.dist]\ndist = true"),
        "cargo-dist package selection must exclude legacy and enable only Reborn"
    );
    assert!(
        wix_manifest.contains("Name='ironclaw'")
            && wix_manifest.contains("Name='ironclaw.exe'")
            && wix_manifest.contains("Source='$(var.CargoTargetBinDir)\\ironclaw.exe'")
            && !wix_manifest.contains("ironclaw-legacy")
            && !root.join("wix/main.wxs").exists(),
        "the MSI definition must install only the canonical Reborn executable"
    );
    let reborn_cli_selector = code_style_workflow
        .lines()
        .find(|line| line.contains("grep -Eq") && line.contains("crates/ironclaw_reborn_cli/"))
        .expect("code style workflow should classify Reborn CLI changes");
    assert!(
        reborn_cli_selector.contains(r"\.github/dist-build-setup\.yml$")
            && reborn_cli_selector.contains(
                r"\.github/workflows/(code_style|ironclaw-release|docker|reborn-release-compile)\.yml$"
            ),
        "release workflow-only PRs must run the Reborn CLI smoke contract"
    );
    assert!(
        code_style_workflow.contains(
            r#"if [[ "${{ needs.changes.outputs.has_reborn_cli }}" == "true" && "${{ needs.reborn-cli-smoke.result }}" != "success" ]]; then"#,
        ),
        "the required Code Style roll-up must propagate workflow-only Reborn CLI smoke failures"
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
