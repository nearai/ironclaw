use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;

use crate::error::BenchError;
use crate::suite::{BenchScore, BenchSuite, BenchTask, TaskSubmission};

// ── Validation helpers ──────────────────────────────────────────────────────

/// Validate that a string is safe for use as a filesystem path component.
/// Allows alphanumerics, hyphens, underscores, dots, and forward slashes (for nested paths).
/// Rejects absolute paths, `..` traversal, and shell metacharacters.
fn is_safe_path_component(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('/')
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

/// Validate that a repo string matches the expected `owner/repo` GitHub format.
fn is_valid_github_repo(repo: &str) -> bool {
    static REPO_PATTERN: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+$").unwrap());
    REPO_PATTERN.is_match(repo)
}

/// Validate that a string looks like a git ref (hex SHA or valid ref name).
fn is_valid_git_ref(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        && !s.contains("..")
}

// ── Types ───────────────────────────────────────────────────────────────────

/// SWE-bench dataset entry (supports both lite and full formats).
#[derive(Debug, Deserialize)]
struct SweBenchEntry {
    instance_id: String,
    repo: String,
    base_commit: String,
    #[serde(default)]
    problem_statement: String,
    #[serde(default)]
    hints_text: Option<String>,
    #[serde(default)]
    test_patch: Option<String>,
    #[serde(default)]
    patch: Option<String>,
    /// JSON-encoded array of test IDs that should go from fail to pass.
    #[serde(default, rename = "FAIL_TO_PASS")]
    fail_to_pass: Option<String>,
    /// JSON-encoded array of test IDs that must continue to pass.
    #[serde(default, rename = "PASS_TO_PASS")]
    pass_to_pass: Option<String>,
    /// Optional test command override (not in standard SWE-bench, but supported).
    #[serde(default)]
    test_cmd: Option<String>,
    /// Version string for the repo at this instance (e.g., "4.2").
    #[serde(default)]
    version: Option<String>,
}

/// Result of running a test command.
struct TestRunResult {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

// ── SweBenchSuite ───────────────────────────────────────────────────────────

/// SWE-bench: real-world software engineering tasks.
///
/// Each task clones a repo at a specific commit, presents the problem statement,
/// and expects the agent to produce a patch. Scoring applies the agent's diff,
/// layers the test patch on top, runs the specified tests, and grades pass/fail
/// per the official SWE-bench protocol (all FAIL_TO_PASS must pass, all
/// PASS_TO_PASS must still pass).
///
/// The runner calls `teardown_task()` before `score()`, so teardown captures the
/// agent's diff to `.patches/{task_id}.agent.patch` before cleaning up the repo.
pub struct SweBenchSuite {
    dataset_path: PathBuf,
    workspace_dir: PathBuf,
    use_docker: bool,
}

impl SweBenchSuite {
    pub fn new(
        dataset_path: impl Into<PathBuf>,
        workspace_dir: impl Into<PathBuf>,
        use_docker: bool,
    ) -> Self {
        Self {
            dataset_path: dataset_path.into(),
            workspace_dir: workspace_dir.into(),
            use_docker,
        }
    }

    /// Directory for saved agent patches: `workspace_dir/.patches/`
    fn patches_dir(&self) -> PathBuf {
        self.workspace_dir.join(".patches")
    }

    /// Path for a specific task's saved agent diff.
    fn agent_patch_path(&self, task_id: &str) -> PathBuf {
        self.patches_dir().join(format!("{task_id}.agent.patch"))
    }
}

// ── Pure helper functions ───────────────────────────────────────────────────

/// Parse a SWE-bench test ID list.
///
/// The field is typically a JSON string containing a JSON array:
///   `"[\"test1\", \"test2\"]"`
/// Also handles the case where it's already a bare JSON array (for robustness).
fn parse_test_id_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // Try parsing as JSON array directly
    if let Ok(ids) = serde_json::from_str::<Vec<String>>(trimmed) {
        return ids;
    }
    // Try parsing as a JSON string containing a JSON array
    if let Ok(inner) = serde_json::from_str::<String>(trimmed) {
        if let Ok(ids) = serde_json::from_str::<Vec<String>>(&inner) {
            return ids;
        }
    }
    Vec::new()
}

/// Extract FAIL_TO_PASS and PASS_TO_PASS test ID lists from task metadata.
///
/// Falls back to extracting test file paths from the test_patch diff headers
/// when explicit lists aren't available.
fn extract_test_ids(task: &BenchTask) -> (Vec<String>, Vec<String>) {
    let f2p = task
        .metadata
        .get("fail_to_pass")
        .and_then(|v| v.as_str())
        .map(parse_test_id_list)
        .unwrap_or_default();

    let p2p = task
        .metadata
        .get("pass_to_pass")
        .and_then(|v| v.as_str())
        .map(parse_test_id_list)
        .unwrap_or_default();

    if !f2p.is_empty() {
        return (f2p, p2p);
    }

    // Fallback: extract test files from test_patch diff headers
    if let Some(patch) = task.metadata.get("test_patch").and_then(|v| v.as_str()) {
        let files = extract_test_files_from_patch(patch);
        return (files, Vec::new());
    }

    (Vec::new(), Vec::new())
}

/// Parse `+++ b/path` headers from a unified diff to find test files.
fn extract_test_files_from_patch(patch: &str) -> Vec<String> {
    static DIFF_PATH: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^\+\+\+ b/(.+)$").unwrap());

    let mut files = Vec::new();
    for line in patch.lines() {
        if let Some(caps) = DIFF_PATH.captures(line) {
            if let Some(path) = caps.get(1) {
                let p = path.as_str().to_string();
                if !files.contains(&p) {
                    files.push(p);
                }
            }
        }
    }
    files
}

/// Infer the test command for a repo when not explicitly provided.
fn default_test_cmd(repo: &str) -> &'static str {
    match repo {
        "django/django" => "./tests/runtests.py --settings=test_sqlite --parallel 1",
        "sympy/sympy" => "bin/test -C --no-colors",
        "sphinx-doc/sphinx" => "tox -epy39 --",
        _ => "pytest --no-header -rN",
    }
}

/// Build a full test command by appending test IDs to the base command.
///
/// For Django, pytest-style test IDs are converted to Django module paths.
/// For other repos, test IDs are passed through unchanged.
fn build_test_command(base_cmd: &str, test_ids: &[String], repo: &str) -> String {
    if test_ids.is_empty() {
        return base_cmd.to_string();
    }

    let ids: Vec<String> = if repo == "django/django" {
        test_ids
            .iter()
            .map(|id| convert_to_django_test_id(id))
            .collect()
    } else {
        test_ids.to_vec()
    };

    // Shell-escape each test ID: pytest IDs contain [], *, ", () etc.
    let escaped: Vec<String> = ids.iter().map(|id| shell_escape(id)).collect();
    format!("{} {}", base_cmd, escaped.join(" "))
}

/// Convert a pytest-style test ID to Django's runtests.py format.
///
///   `tests/admin_views/tests.py::AdminViewTest::test_foo`
///   -> `admin_views.tests.AdminViewTest.test_foo`
///
/// If the ID doesn't contain `::`, returns it unchanged (already in Django format).
fn convert_to_django_test_id(pytest_id: &str) -> String {
    if !pytest_id.contains("::") {
        return pytest_id.to_string();
    }

    let parts: Vec<&str> = pytest_id.splitn(2, "::").collect();
    let file_path = parts[0];
    let test_part = parts.get(1).copied().unwrap_or("");

    // Strip leading `tests/` prefix and `.py` suffix, convert `/` to `.`
    let module = file_path
        .strip_prefix("tests/")
        .unwrap_or(file_path)
        .strip_suffix(".py")
        .unwrap_or(file_path)
        .replace('/', ".");

    if test_part.is_empty() {
        module
    } else {
        format!("{}.{}", module, test_part.replace("::", "."))
    }
}

/// Shell-escape a string by wrapping in single quotes.
///
/// Single quotes inside the string are handled with the `'\''` idiom
/// (end quote, escaped literal quote, start quote).
fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // If no special chars, skip quoting
    if s.chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':'))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Truncate combined test output to a maximum byte length, keeping the tail.
fn truncate_output(stdout: &str, stderr: &str, max_len: usize) -> String {
    let combined = format!("=== stdout ===\n{stdout}\n=== stderr ===\n{stderr}");
    if combined.len() <= max_len {
        return combined;
    }
    if max_len == 0 {
        return "... (truncated)".to_string();
    }
    let truncated = &combined[combined.len().saturating_sub(max_len)..];
    if let Some(pos) = truncated.find('\n') {
        format!("... (truncated)\n{}", &truncated[pos + 1..])
    } else {
        format!("... (truncated)\n{truncated}")
    }
}

// ── Docker helpers ──────────────────────────────────────────────────────────

/// Path to the Dockerfile baked into this crate.
const DOCKERFILE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docker/swe-bench.Dockerfile");

/// Path to the entrypoint script baked into this crate.
const ENTRYPOINT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docker/swe-bench-run.sh");

/// Build a deterministic Docker image tag from a repo and base commit.
///
/// Format: `swebench-{repo_slug}-{commit_prefix}` where the repo slug
/// replaces `/` with `-` and the commit is truncated to 10 chars.
fn docker_image_tag(repo: &str, base_commit: &str) -> String {
    let slug = repo.replace('/', "-").to_lowercase();
    let prefix_len = base_commit.len().min(10);
    let commit_prefix = &base_commit[..prefix_len];
    format!("swebench-{slug}-{commit_prefix}")
}

/// Map a repo (and optional version) to a Python version.
///
/// SWE-bench Lite repos are mostly fine with 3.9, but a few need newer or
/// older versions. This can be extended as needed.
fn python_version_for_repo(repo: &str, version: Option<&str>) -> &'static str {
    match (repo, version) {
        ("django/django", Some(v)) if v.starts_with("4.") || v.starts_with("5.") => "3.11",
        ("django/django", _) => "3.9",
        ("sphinx-doc/sphinx", _) => "3.9",
        ("sympy/sympy", _) => "3.9",
        ("scikit-learn/scikit-learn", _) => "3.9",
        ("matplotlib/matplotlib", _) => "3.9",
        ("astropy/astropy", _) => "3.9",
        ("psf/requests", _) => "3.9",
        ("pylint-dev/pylint", _) => "3.9",
        ("pytest-dev/pytest", _) => "3.9",
        _ => "3.9",
    }
}

/// Read an exit code from a file in a directory.
///
/// Returns `None` if the file doesn't exist or can't be parsed.
fn read_exit_code(dir: &Path, filename: &str) -> Option<i32> {
    let path = dir.join(filename);
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<i32>()
        .ok()
}

/// Check whether a Docker image with the given tag exists locally.
async fn docker_image_exists(tag: &str) -> bool {
    tokio::process::Command::new("docker")
        .args(["image", "inspect", tag])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Build a Docker image from a cloned repo directory.
///
/// Uses the Dockerfile from this crate with the task directory as build context.
/// The image is tagged deterministically so subsequent runs skip the build.
async fn build_docker_image(
    task_dir: &Path,
    image_tag: &str,
    python_version: &str,
    task_id: &str,
) -> Result<(), BenchError> {
    tracing::info!(task_id, image_tag, python_version, "building Docker image");

    let output = tokio::process::Command::new("docker")
        .args([
            "build",
            "-f",
            DOCKERFILE_PATH,
            "--build-arg",
            &format!("PYTHON_VERSION={python_version}"),
            "-t",
            image_tag,
            ".",
        ])
        .current_dir(task_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| BenchError::TaskFailed {
            task_id: task_id.to_string(),
            reason: format!("docker build failed to start: {e}"),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BenchError::TaskFailed {
            task_id: task_id.to_string(),
            reason: format!("docker build failed:\n{stderr}"),
        });
    }

    tracing::info!(task_id, image_tag, "Docker image built successfully");
    Ok(())
}

/// Resolve truncated test IDs by running `pytest --collect-only` inside a Docker container.
///
/// This ensures we use the correct pytest version and dependencies from the image.
async fn resolve_test_ids_docker(image_tag: &str, ids: Vec<String>) -> Vec<String> {
    let (complete, truncated): (Vec<_>, Vec<_>) = ids
        .into_iter()
        .partition(|id| !id.contains('[') || id.ends_with(']'));

    if truncated.is_empty() {
        return complete;
    }

    let test_files: std::collections::HashSet<&str> = truncated
        .iter()
        .filter_map(|id| id.split("::").next())
        .collect();

    let mut cmd_args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--network=none".to_string(),
        image_tag.to_string(),
        "python".to_string(),
        "-m".to_string(),
        "pytest".to_string(),
        "--collect-only".to_string(),
        "-q".to_string(),
    ];
    cmd_args.extend(test_files.into_iter().map(String::from));

    let output = tokio::process::Command::new("docker")
        .args(&cmd_args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    let collected: Vec<String> = match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains("::"))
            .map(|l| l.trim().to_string())
            .collect(),
        Err(_) => {
            let mut result = complete;
            result.extend(truncated);
            return result;
        }
    };

    let mut result = complete;
    for trunc in &truncated {
        let matched: Vec<&String> = collected.iter().filter(|c| c.starts_with(trunc)).collect();
        if matched.is_empty() {
            tracing::debug!("dropping unresolvable truncated test ID: {trunc}");
        } else {
            for m in matched {
                result.push(m.clone());
            }
        }
    }
    result
}

/// Full Docker-based scoring path.
///
/// Uses file-based IPC via a mounted staging directory to avoid shell escaping
/// headaches. The entrypoint script runs inside the container, reading inputs
/// and writing results to /work/.
async fn score_docker(
    task: &BenchTask,
    agent_patch: &str,
    test_patch: &str,
    image_tag: &str,
    base_commit: &str,
    repo: &str,
) -> Result<BenchScore, BenchError> {
    // 1. Extract and resolve test IDs
    let (f2p, p2p) = extract_test_ids(task);
    if f2p.is_empty() {
        return Ok(BenchScore::fail(
            "no FAIL_TO_PASS test IDs found; cannot verify",
        ));
    }
    let f2p = resolve_test_ids_docker(image_tag, f2p).await;
    let p2p = resolve_test_ids_docker(image_tag, p2p).await;

    // 2. Build test commands
    let base_cmd = task
        .metadata
        .get("test_cmd")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| default_test_cmd(repo));

    let f2p_cmd = build_test_command(base_cmd, &f2p, repo);
    let p2p_cmd = if p2p.is_empty() {
        String::new()
    } else {
        build_test_command(base_cmd, &p2p, repo)
    };

    // 3. Create staging directory with input files
    let staging = tempfile::tempdir().map_err(|e| BenchError::Scoring {
        task_id: task.id.clone(),
        reason: format!("failed to create staging dir: {e}"),
    })?;
    let staging_path = staging.path();

    std::fs::write(staging_path.join("agent.patch"), agent_patch).map_err(|e| {
        BenchError::Scoring {
            task_id: task.id.clone(),
            reason: format!("failed to write agent.patch: {e}"),
        }
    })?;
    std::fs::write(staging_path.join("test.patch"), test_patch).map_err(|e| {
        BenchError::Scoring {
            task_id: task.id.clone(),
            reason: format!("failed to write test.patch: {e}"),
        }
    })?;
    std::fs::write(staging_path.join("base_commit"), base_commit).map_err(|e| {
        BenchError::Scoring {
            task_id: task.id.clone(),
            reason: format!("failed to write base_commit: {e}"),
        }
    })?;
    std::fs::write(staging_path.join("f2p_cmd"), &f2p_cmd).map_err(|e| BenchError::Scoring {
        task_id: task.id.clone(),
        reason: format!("failed to write f2p_cmd: {e}"),
    })?;
    std::fs::write(staging_path.join("p2p_cmd"), &p2p_cmd).map_err(|e| BenchError::Scoring {
        task_id: task.id.clone(),
        reason: format!("failed to write p2p_cmd: {e}"),
    })?;

    // 4. Run the container
    let staging_mount = format!("{}:/work", staging_path.display());
    let entrypoint_mount = format!("{ENTRYPOINT_PATH}:/entrypoint.sh:ro");

    tracing::info!(
        task_id = %task.id,
        image_tag,
        "running Docker scoring: F2P={f2p_cmd}"
    );

    let child = tokio::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--network=none",
            "--memory=4g",
            "-v",
            &staging_mount,
            "-v",
            &entrypoint_mount,
            image_tag,
            "bash",
            "/entrypoint.sh",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| BenchError::Scoring {
            task_id: task.id.clone(),
            reason: format!("failed to start docker run: {e}"),
        })?;

    // 5. Wait with timeout
    let output = match tokio::time::timeout(TEST_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Ok(BenchScore::fail(format!("docker run failed: {e}")));
        }
        Err(_) => {
            return Ok(BenchScore::fail(format!(
                "docker run timed out after {}s",
                TEST_TIMEOUT.as_secs()
            )));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(BenchScore::fail(format!(
            "docker run exited with {}: {stderr}",
            output.status
        )));
    }

    // 6. Read results
    if let Ok(error) = std::fs::read_to_string(staging_path.join("error")) {
        if !error.trim().is_empty() {
            return Ok(BenchScore::fail(format!(
                "container error: {}",
                error.trim()
            )));
        }
    }

    let f2p_exit = read_exit_code(staging_path, "f2p_exit");
    let p2p_exit = read_exit_code(staging_path, "p2p_exit");

    let f2p_passed = f2p_exit == Some(0);
    let p2p_passed = p2p_exit == Some(0);

    // 7. Grade
    if f2p_passed && p2p_passed {
        Ok(BenchScore::pass())
    } else {
        let mut reasons = Vec::new();
        if !f2p_passed {
            let stdout =
                std::fs::read_to_string(staging_path.join("f2p_stdout")).unwrap_or_default();
            let stderr =
                std::fs::read_to_string(staging_path.join("f2p_stderr")).unwrap_or_default();
            let detail = truncate_output(&stdout, &stderr, 2000);
            reasons.push(format!(
                "FAIL_TO_PASS tests did not pass (exit={}):\n{detail}",
                f2p_exit.map_or("?".to_string(), |c| c.to_string())
            ));
        }
        if !p2p_passed {
            let stdout =
                std::fs::read_to_string(staging_path.join("p2p_stdout")).unwrap_or_default();
            let stderr =
                std::fs::read_to_string(staging_path.join("p2p_stderr")).unwrap_or_default();
            let detail = truncate_output(&stdout, &stderr, 2000);
            reasons.push(format!(
                "PASS_TO_PASS tests regressed (exit={}):\n{detail}",
                p2p_exit.map_or("?".to_string(), |c| c.to_string())
            ));
        }
        Ok(BenchScore::fail(reasons.join("\n\n")))
    }
}

// ── Git / test execution helpers ────────────────────────────────────────────

/// Timeout for test suite execution (10 minutes).
const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Ensure the task repo is at a clean state on the given base commit.
async fn ensure_clean_checkout(
    task_dir: &Path,
    base_commit: &str,
    task_id: &str,
) -> Result<(), BenchError> {
    let _ = tokio::process::Command::new("git")
        .args(["reset", "--hard", base_commit])
        .current_dir(task_dir)
        .output()
        .await;

    let _ = tokio::process::Command::new("git")
        .args(["clean", "-fdx"])
        .current_dir(task_dir)
        .output()
        .await;

    // Verify clean
    let status = tokio::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(task_dir)
        .output()
        .await
        .map_err(|e| BenchError::Scoring {
            task_id: task_id.to_string(),
            reason: format!("git status failed: {e}"),
        })?;

    let porcelain = String::from_utf8_lossy(&status.stdout);
    if !porcelain.trim().is_empty() {
        return Err(BenchError::Scoring {
            task_id: task_id.to_string(),
            reason: format!("repo not clean after reset: {}", porcelain.trim()),
        });
    }

    Ok(())
}

/// Apply a patch to the repo via stdin. Falls back to `--3way` if straight apply fails.
async fn apply_patch_to_repo(task_dir: &Path, patch: &str, label: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    // Try straight apply first
    let mut child = tokio::process::Command::new("git")
        .args(["apply", "--verbose"])
        .current_dir(task_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{label}: failed to spawn git apply: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(patch.as_bytes())
            .await
            .map_err(|e| format!("{label}: failed to write patch to stdin: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("{label}: git apply failed: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    // Fall back to --3way for fuzzy matching
    tracing::debug!(
        "{label}: straight git apply failed, trying --3way. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut child = tokio::process::Command::new("git")
        .args(["apply", "--3way", "--verbose"])
        .current_dir(task_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{label}: failed to spawn git apply --3way: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(patch.as_bytes())
            .await
            .map_err(|e| format!("{label}: failed to write patch to stdin (3way): {e}"))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("{label}: git apply --3way failed: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("{label}: patch failed to apply: {stderr}"))
}

/// Reset the repo to a clean state (best-effort cleanup after scoring).
async fn reset_repo(task_dir: &Path) {
    let _ = tokio::process::Command::new("git")
        .args(["checkout", "."])
        .current_dir(task_dir)
        .output()
        .await;
    let _ = tokio::process::Command::new("git")
        .args(["clean", "-fdx"])
        .current_dir(task_dir)
        .output()
        .await;
    let _ = tokio::process::Command::new("git")
        .args(["reset", "HEAD"])
        .current_dir(task_dir)
        .output()
        .await;
}

/// Resolve truncated test IDs against the actual collected tests.
///
/// SWE-bench dataset sometimes has truncated parameterized test IDs like
/// `test_foo[TypeError-TypeError-*1` (missing closing `]`). We collect
/// the full test list and prefix-match to recover the real IDs.
async fn resolve_test_ids(task_dir: &Path, ids: Vec<String>) -> Vec<String> {
    // Split into complete and truncated IDs
    let (complete, truncated): (Vec<_>, Vec<_>) = ids
        .into_iter()
        .partition(|id| !id.contains('[') || id.ends_with(']'));

    if truncated.is_empty() {
        return complete;
    }

    // Collect all test IDs from the test files referenced by truncated IDs
    let test_files: std::collections::HashSet<&str> = truncated
        .iter()
        .filter_map(|id| id.split("::").next())
        .collect();

    let collect_args: Vec<String> = test_files.into_iter().map(String::from).collect();
    let mut cmd_args = vec!["--collect-only".to_string(), "-q".to_string()];
    cmd_args.extend(collect_args);

    let output = tokio::process::Command::new("python")
        .args(["-m", "pytest"])
        .args(&cmd_args)
        .current_dir(task_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    let collected: Vec<String> = match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains("::"))
            .map(|l| l.trim().to_string())
            .collect(),
        Err(_) => {
            // Can't collect, return what we have (truncated IDs will fail at pytest)
            let mut result = complete;
            result.extend(truncated);
            return result;
        }
    };

    // Match truncated IDs to collected IDs by prefix
    let mut result = complete;
    for trunc in &truncated {
        let matched: Vec<&String> = collected.iter().filter(|c| c.starts_with(trunc)).collect();
        if matched.is_empty() {
            tracing::debug!("dropping unresolvable truncated test ID: {trunc}");
        } else {
            for m in matched {
                result.push(m.clone());
            }
        }
    }
    result
}

/// Run a test command in the task directory with a 10-minute timeout.
async fn run_tests(task_dir: &Path, test_cmd: &str) -> TestRunResult {
    let child = tokio::process::Command::new("bash")
        .args(["-c", test_cmd])
        .current_dir(task_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return TestRunResult {
                exit_code: None,
                stdout: String::new(),
                stderr: format!("failed to spawn test command: {e}"),
                timed_out: false,
            };
        }
    };

    match tokio::time::timeout(TEST_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => TestRunResult {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            timed_out: false,
        },
        Ok(Err(e)) => TestRunResult {
            exit_code: None,
            stdout: String::new(),
            stderr: format!("test command failed: {e}"),
            timed_out: false,
        },
        Err(_) => TestRunResult {
            exit_code: None,
            stdout: String::new(),
            stderr: format!("test timed out after {}s", TEST_TIMEOUT.as_secs()),
            timed_out: true,
        },
    }
}

// ── BenchSuite impl ─────────────────────────────────────────────────────────

#[async_trait]
impl BenchSuite for SweBenchSuite {
    fn name(&self) -> &str {
        "SWE-bench Pro"
    }

    fn id(&self) -> &str {
        "swe_bench"
    }

    async fn load_tasks(&self) -> Result<Vec<BenchTask>, BenchError> {
        let file = std::fs::File::open(&self.dataset_path)?;
        let reader = std::io::BufReader::new(file);
        let mut tasks = Vec::new();

        for (line_num, line) in reader.lines().enumerate() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: SweBenchEntry = serde_json::from_str(trimmed).map_err(|e| {
                BenchError::Config(format!("swe_bench line {}: {}", line_num + 1, e))
            })?;

            if !is_safe_path_component(&entry.instance_id) {
                return Err(BenchError::Config(format!(
                    "swe_bench line {}: unsafe instance_id \"{}\"",
                    line_num + 1,
                    entry.instance_id,
                )));
            }
            if !is_valid_github_repo(&entry.repo) {
                return Err(BenchError::Config(format!(
                    "swe_bench line {}: invalid repo format \"{}\"",
                    line_num + 1,
                    entry.repo,
                )));
            }
            if !is_valid_git_ref(&entry.base_commit) {
                return Err(BenchError::Config(format!(
                    "swe_bench line {}: invalid base_commit \"{}\"",
                    line_num + 1,
                    entry.base_commit,
                )));
            }

            let task_dir = self.workspace_dir.join(&entry.instance_id);

            let metadata = serde_json::json!({
                "repo": entry.repo,
                "base_commit": entry.base_commit,
                "test_patch": entry.test_patch,
                "gold_patch": entry.patch,
                "fail_to_pass": entry.fail_to_pass,
                "pass_to_pass": entry.pass_to_pass,
                "test_cmd": entry.test_cmd,
                "version": entry.version,
                "use_docker": self.use_docker,
                "workspace_dir": self.workspace_dir.to_string_lossy(),
            });

            let mut prompt = format!(
                "You are fixing a bug in the **{repo}** repository (commit `{commit}`).\n\
                 The repository is checked out at: `{path}`\n\n\
                 ## Problem Statement\n\n\
                 {problem}",
                repo = entry.repo,
                commit = entry.base_commit,
                path = task_dir.display(),
                problem = entry.problem_statement,
            );

            if let Some(ref hints) = entry.hints_text {
                if !hints.trim().is_empty() {
                    prompt.push_str(&format!("\n\n## Hints\n\n{hints}"));
                }
            }

            prompt.push_str(
                "\n\n## Instructions\n\n\
                 - Fix the issue described above by modifying the source code.\n\
                 - Do NOT modify any test files.\n\
                 - Make minimal, focused changes.\n\
                 - Use the shell, read_file, write_file, list_dir, and apply_patch tools to explore and modify the code.",
            );

            tasks.push(BenchTask {
                id: entry.instance_id,
                prompt,
                context: None,
                resources: vec![],
                tags: vec![format!("repo-{}", entry.repo.replace('/', "-"))],
                expected_turns: None,
                timeout: None,
                metadata,
            });
        }

        Ok(tasks)
    }

    async fn setup_task(&self, task: &BenchTask) -> Result<(), BenchError> {
        let repo = task
            .metadata
            .get("repo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BenchError::TaskFailed {
                task_id: task.id.clone(),
                reason: "missing repo in metadata".to_string(),
            })?;
        let base_commit = task
            .metadata
            .get("base_commit")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BenchError::TaskFailed {
                task_id: task.id.clone(),
                reason: "missing base_commit in metadata".to_string(),
            })?;

        let task_dir = self.workspace_dir.join(&task.id);

        // Clone repo if not already present
        if !task_dir.exists() {
            let repo_url = format!("https://github.com/{}.git", repo);
            let output = tokio::process::Command::new("git")
                .args([
                    "clone",
                    "--depth",
                    "1",
                    &repo_url,
                    &task_dir.to_string_lossy(),
                ])
                .output()
                .await
                .map_err(|e| BenchError::TaskFailed {
                    task_id: task.id.clone(),
                    reason: format!("git clone failed: {e}"),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(BenchError::TaskFailed {
                    task_id: task.id.clone(),
                    reason: format!("git clone failed: {stderr}"),
                });
            }
        }

        // Checkout the base commit
        let output = tokio::process::Command::new("git")
            .args(["checkout", base_commit])
            .current_dir(&task_dir)
            .output()
            .await
            .map_err(|e| BenchError::TaskFailed {
                task_id: task.id.clone(),
                reason: format!("git checkout failed: {e}"),
            })?;

        if !output.status.success() {
            // Shallow clone might not have the commit; fetch more history
            let _ = tokio::process::Command::new("git")
                .args(["fetch", "--unshallow"])
                .current_dir(&task_dir)
                .output()
                .await;

            let output = tokio::process::Command::new("git")
                .args(["checkout", base_commit])
                .current_dir(&task_dir)
                .output()
                .await
                .map_err(|e| BenchError::TaskFailed {
                    task_id: task.id.clone(),
                    reason: format!("git checkout retry failed: {e}"),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(BenchError::TaskFailed {
                    task_id: task.id.clone(),
                    reason: format!("git checkout failed: {stderr}"),
                });
            }
        }

        // Build Docker image if configured (cached by tag, skips if already built)
        if self.use_docker {
            let version = task.metadata.get("version").and_then(|v| v.as_str());
            let python_version = python_version_for_repo(repo, version);
            let image_tag = docker_image_tag(repo, base_commit);

            if !docker_image_exists(&image_tag).await {
                build_docker_image(&task_dir, &image_tag, python_version, &task.id).await?;
            } else {
                tracing::info!(task_id = %task.id, image_tag, "Docker image already exists, skipping build");
            }
        }

        Ok(())
    }

    async fn teardown_task(&self, task: &BenchTask) -> Result<(), BenchError> {
        let task_dir = self.workspace_dir.join(&task.id);
        if !task_dir.exists() {
            return Ok(());
        }

        // Create patches directory
        let patches_dir = self.patches_dir();
        std::fs::create_dir_all(&patches_dir).map_err(|e| BenchError::TaskFailed {
            task_id: task.id.clone(),
            reason: format!("failed to create patches dir: {e}"),
        })?;

        // Stage everything so we capture new files in the diff
        let _ = tokio::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&task_dir)
            .output()
            .await;

        // Capture the full diff of all agent changes vs HEAD
        let diff_output = tokio::process::Command::new("git")
            .args(["diff", "--cached", "HEAD"])
            .current_dir(&task_dir)
            .output()
            .await;

        if let Ok(output) = diff_output {
            let patch = String::from_utf8_lossy(&output.stdout);
            let patch_path = self.agent_patch_path(&task.id);
            if let Err(e) = std::fs::write(&patch_path, patch.as_bytes()) {
                tracing::warn!(task_id = %task.id, "failed to write agent patch: {e}");
            }
        }

        // Unstage so cleanup works correctly
        let _ = tokio::process::Command::new("git")
            .args(["reset", "HEAD"])
            .current_dir(&task_dir)
            .output()
            .await;

        // Reset working tree
        let _ = tokio::process::Command::new("git")
            .args(["checkout", "."])
            .current_dir(&task_dir)
            .output()
            .await;

        // Remove untracked files
        let _ = tokio::process::Command::new("git")
            .args(["clean", "-fdx"])
            .current_dir(&task_dir)
            .output()
            .await;

        Ok(())
    }

    async fn score(
        &self,
        task: &BenchTask,
        _submission: &TaskSubmission,
    ) -> Result<BenchScore, BenchError> {
        let task_dir = self.workspace_dir.join(&task.id);
        let repo = task
            .metadata
            .get("repo")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let base_commit = task
            .metadata
            .get("base_commit")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BenchError::Scoring {
                task_id: task.id.clone(),
                reason: "missing base_commit in metadata".to_string(),
            })?;

        // 1. Read saved agent patch (written by teardown_task)
        let patch_path = self.agent_patch_path(&task.id);
        let agent_patch = match std::fs::read_to_string(&patch_path) {
            Ok(p) => p,
            Err(_) => {
                return Ok(BenchScore::fail(
                    "no agent patch found (teardown may not have run)",
                ));
            }
        };

        // 2. Empty patch = agent made no changes
        if agent_patch.trim().is_empty() {
            return Ok(BenchScore::fail("agent produced no code changes"));
        }

        // 3. Must have a test patch to verify the solution
        let test_patch = task.metadata.get("test_patch").and_then(|v| v.as_str());
        let test_patch = match test_patch {
            Some(p) if !p.trim().is_empty() => p,
            _ => {
                return Ok(BenchScore::fail(
                    "no test_patch available; cannot verify solution",
                ));
            }
        };

        // 4. Branch: Docker or local scoring
        if self.use_docker {
            let image_tag = docker_image_tag(repo, base_commit);
            return score_docker(
                task,
                &agent_patch,
                test_patch,
                &image_tag,
                base_commit,
                repo,
            )
            .await;
        }

        // ── Local scoring path (no Docker) ──────────────────────────────

        // 4. Clean checkout to base commit
        ensure_clean_checkout(&task_dir, base_commit, &task.id).await?;

        // 5. Apply agent's patch
        if let Err(reason) = apply_patch_to_repo(&task_dir, &agent_patch, "agent_patch").await {
            reset_repo(&task_dir).await;
            return Ok(BenchScore::fail(format!(
                "agent patch failed to apply: {reason}"
            )));
        }

        // 6. Apply test patch on top
        if let Err(reason) = apply_patch_to_repo(&task_dir, test_patch, "test_patch").await {
            reset_repo(&task_dir).await;
            return Ok(BenchScore::fail(format!(
                "test patch failed to apply on top of agent changes: {reason}"
            )));
        }

        // 7. Extract and resolve test IDs (fix truncated parameterized IDs)
        let (f2p, p2p) = extract_test_ids(task);
        if f2p.is_empty() {
            reset_repo(&task_dir).await;
            return Ok(BenchScore::fail(
                "no FAIL_TO_PASS test IDs found; cannot verify",
            ));
        }
        let f2p = resolve_test_ids(&task_dir, f2p).await;
        let p2p = resolve_test_ids(&task_dir, p2p).await;

        // 8. Determine test command
        let base_cmd = task
            .metadata
            .get("test_cmd")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| default_test_cmd(repo));

        // 9. Run FAIL_TO_PASS tests (these should now pass after the fix)
        let f2p_cmd = build_test_command(base_cmd, &f2p, repo);
        tracing::info!(task_id = %task.id, "running FAIL_TO_PASS tests: {f2p_cmd}");
        let f2p_result = run_tests(&task_dir, &f2p_cmd).await;
        let f2p_passed = f2p_result.exit_code == Some(0) && !f2p_result.timed_out;

        // 10. Run PASS_TO_PASS tests (these must still pass, no regressions)
        let (p2p_passed, p2p_result) = if p2p.is_empty() {
            (true, None)
        } else {
            let p2p_cmd = build_test_command(base_cmd, &p2p, repo);
            tracing::info!(task_id = %task.id, "running PASS_TO_PASS tests: {p2p_cmd}");
            let result = run_tests(&task_dir, &p2p_cmd).await;
            let passed = result.exit_code == Some(0) && !result.timed_out;
            (passed, Some(result))
        };

        // 11. Cleanup
        reset_repo(&task_dir).await;

        // 12. Grade: SWE-bench is binary, all or nothing
        if f2p_passed && p2p_passed {
            Ok(BenchScore::pass())
        } else {
            let mut reasons = Vec::new();
            if !f2p_passed {
                let detail = truncate_output(&f2p_result.stdout, &f2p_result.stderr, 2000);
                reasons.push(format!("FAIL_TO_PASS tests did not pass:\n{detail}"));
            }
            if !p2p_passed {
                if let Some(ref result) = p2p_result {
                    let detail = truncate_output(&result.stdout, &result.stderr, 2000);
                    reasons.push(format!("PASS_TO_PASS tests regressed:\n{detail}"));
                } else {
                    reasons.push("PASS_TO_PASS tests regressed".to_string());
                }
            }
            Ok(BenchScore::fail(reasons.join("\n\n")))
        }
    }

    fn task_tools(&self, task: &BenchTask) -> Vec<Arc<dyn ironclaw::tools::Tool>> {
        let task_dir = self.workspace_dir.join(&task.id);
        vec![
            Arc::new(ironclaw::tools::builtin::ShellTool::new().with_working_dir(task_dir.clone())),
            Arc::new(ironclaw::tools::builtin::ReadFileTool::new().with_base_dir(task_dir.clone())),
            Arc::new(
                ironclaw::tools::builtin::WriteFileTool::new().with_base_dir(task_dir.clone()),
            ),
            Arc::new(ironclaw::tools::builtin::ListDirTool::new().with_base_dir(task_dir.clone())),
            Arc::new(ironclaw::tools::builtin::ApplyPatchTool::new().with_base_dir(task_dir)),
        ]
    }

    fn system_prompt(&self) -> Option<String> {
        Some(
            "You are an expert software engineer tasked with fixing a bug in an \
             open-source repository.\n\n\
             Your workflow:\n\
             1. Read the problem statement carefully\n\
             2. Explore the relevant source files using read_file and list_dir\n\
             3. Use shell to run grep/git commands for deeper understanding\n\
             4. Make targeted fixes using write_file or apply_patch\n\
             5. Verify your changes don't break anything obvious\n\n\
             Rules:\n\
             - Make minimal, focused changes. Fix only what's broken.\n\
             - Do NOT modify test files.\n\
             - Do NOT add new dependencies.\n\
             - When done, explain what you changed and why."
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // ── Task loading tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_swe_bench_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "django__django-12345", "repo": "django/django", "base_commit": "abc123", "problem_statement": "Fix the ORM bug"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, dir.path().join("ws"), false);
        let tasks = suite.load_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "django__django-12345");
        assert!(tasks[0].tags.contains(&"repo-django-django".to_string()));
        // Prompt should include structured sections
        assert!(tasks[0].prompt.contains("## Problem Statement"));
        assert!(tasks[0].prompt.contains("Fix the ORM bug"));
        assert!(tasks[0].prompt.contains("## Instructions"));
    }

    #[tokio::test]
    async fn test_swe_bench_load_with_new_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "django__django-99", "repo": "django/django", "base_commit": "abc", "problem_statement": "Bug", "FAIL_TO_PASS": "[\"tests.test_foo\"]", "PASS_TO_PASS": "[\"tests.test_bar\"]", "version": "4.2"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, dir.path().join("ws"), false);
        let tasks = suite.load_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);

        let meta = &tasks[0].metadata;
        assert_eq!(
            meta.get("fail_to_pass").and_then(|v| v.as_str()),
            Some("[\"tests.test_foo\"]")
        );
        assert_eq!(
            meta.get("pass_to_pass").and_then(|v| v.as_str()),
            Some("[\"tests.test_bar\"]")
        );
        assert_eq!(meta.get("version").and_then(|v| v.as_str()), Some("4.2"));
    }

    #[tokio::test]
    async fn test_swe_bench_scoring_no_patch_file() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "s1", "repo": "org/repo", "base_commit": "abc", "problem_statement": "Fix bug"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, &ws, false);
        let tasks = suite.load_tasks().await.unwrap();

        let submission = TaskSubmission {
            response: String::new(),
            conversation: vec![],
            tool_calls: vec![],
            error: None,
        };
        // No patch file exists, so score should be 0.0
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 0.0);
        assert!(
            score
                .details
                .as_deref()
                .unwrap_or("")
                .contains("no agent patch")
        );
    }

    #[tokio::test]
    async fn test_swe_bench_scoring_empty_patch() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "s2", "repo": "org/repo", "base_commit": "abc", "problem_statement": "Fix bug"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, &ws, false);
        let tasks = suite.load_tasks().await.unwrap();

        // Create an empty patch file
        let patches_dir = suite.patches_dir();
        std::fs::create_dir_all(&patches_dir).unwrap();
        std::fs::write(suite.agent_patch_path("s2"), "").unwrap();

        let submission = TaskSubmission {
            response: "I made changes".to_string(),
            conversation: vec![],
            tool_calls: vec![],
            error: None,
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 0.0);
        assert!(
            score
                .details
                .as_deref()
                .unwrap_or("")
                .contains("no code changes")
        );
    }

    #[tokio::test]
    async fn test_swe_bench_scoring_no_test_patch() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        // Entry has no test_patch
        writeln!(
            file,
            r#"{{"instance_id": "s3", "repo": "org/repo", "base_commit": "abc", "problem_statement": "Fix bug"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, &ws, false);
        let tasks = suite.load_tasks().await.unwrap();

        // Create a non-empty patch file
        let patches_dir = suite.patches_dir();
        std::fs::create_dir_all(&patches_dir).unwrap();
        std::fs::write(suite.agent_patch_path("s3"), "diff --git a/foo b/foo\n").unwrap();

        let submission = TaskSubmission {
            response: "fixed it".to_string(),
            conversation: vec![],
            tool_calls: vec![],
            error: None,
        };
        let score = suite.score(&tasks[0], &submission).await.unwrap();
        assert_eq!(score.value, 0.0);
        assert!(
            score
                .details
                .as_deref()
                .unwrap_or("")
                .contains("no test_patch")
        );
    }

    // ── Validation tests (unchanged) ────────────────────────────────────

    #[test]
    fn test_is_safe_path_component() {
        assert!(is_safe_path_component("django__django-12345"));
        assert!(is_safe_path_component("org/repo"));
        assert!(is_safe_path_component("abc123"));
        assert!(!is_safe_path_component(""));
        assert!(!is_safe_path_component("../../etc/passwd"));
        assert!(!is_safe_path_component("/etc/passwd"));
        assert!(!is_safe_path_component("foo;rm -rf /"));
        assert!(!is_safe_path_component("foo bar"));
    }

    #[test]
    fn test_is_valid_github_repo() {
        assert!(is_valid_github_repo("django/django"));
        assert!(is_valid_github_repo("org/repo-name"));
        assert!(is_valid_github_repo("Org.Name/Repo_v2"));
        assert!(!is_valid_github_repo(""));
        assert!(!is_valid_github_repo("no-slash"));
        assert!(!is_valid_github_repo("too/many/slashes"));
        assert!(!is_valid_github_repo("spa ce/repo"));
    }

    #[test]
    fn test_is_valid_git_ref() {
        assert!(is_valid_git_ref("abc123"));
        assert!(is_valid_git_ref("deadbeef0123456789abcdef0123456789abcdef"));
        assert!(is_valid_git_ref("v1.2.3"));
        assert!(is_valid_git_ref("main"));
        assert!(!is_valid_git_ref(""));
        assert!(!is_valid_git_ref("bad..ref"));
        assert!(!is_valid_git_ref("has space"));
        assert!(!is_valid_git_ref("semi;colon"));
    }

    #[tokio::test]
    async fn test_swe_bench_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "../../etc/passwd", "repo": "org/repo", "base_commit": "abc", "problem_statement": "evil"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, "/tmp/swe-test", false);
        let err = suite.load_tasks().await.unwrap_err();
        assert!(err.to_string().contains("unsafe instance_id"));
    }

    #[tokio::test]
    async fn test_swe_bench_rejects_bad_repo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("swe.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"instance_id": "task1", "repo": "not-a-repo-format", "base_commit": "abc", "problem_statement": "bad"}}"#
        )
        .unwrap();

        let suite = SweBenchSuite::new(&path, "/tmp/swe-test", false);
        let err = suite.load_tasks().await.unwrap_err();
        assert!(err.to_string().contains("invalid repo format"));
    }

    // ── Pure helper function tests ──────────────────────────────────────

    #[test]
    fn test_parse_test_id_list_json_array() {
        let ids = parse_test_id_list(r#"["test1", "test2", "test3"]"#);
        assert_eq!(ids, vec!["test1", "test2", "test3"]);
    }

    #[test]
    fn test_parse_test_id_list_json_string_containing_array() {
        // SWE-bench stores these as JSON strings wrapping JSON arrays
        let ids = parse_test_id_list(r#""[\"test1\", \"test2\"]""#);
        assert_eq!(ids, vec!["test1", "test2"]);
    }

    #[test]
    fn test_parse_test_id_list_empty() {
        assert!(parse_test_id_list("").is_empty());
        assert!(parse_test_id_list("[]").is_empty());
        assert!(parse_test_id_list(r#""[]""#).is_empty());
    }

    #[test]
    fn test_parse_test_id_list_garbage() {
        assert!(parse_test_id_list("not json at all").is_empty());
        assert!(parse_test_id_list("{\"key\": \"value\"}").is_empty());
    }

    #[test]
    fn test_extract_test_files_from_patch() {
        let patch = "\
diff --git a/tests/test_foo.py b/tests/test_foo.py
--- a/tests/test_foo.py
+++ b/tests/test_foo.py
@@ -1,3 +1,4 @@
 line1
+new line
diff --git a/tests/test_bar.py b/tests/test_bar.py
--- a/tests/test_bar.py
+++ b/tests/test_bar.py
@@ -1,2 +1,3 @@
 line1
+another new line
";
        let files = extract_test_files_from_patch(patch);
        assert_eq!(files, vec!["tests/test_foo.py", "tests/test_bar.py"]);
    }

    #[test]
    fn test_extract_test_files_no_duplicates() {
        let patch = "+++ b/tests/test_foo.py\n+++ b/tests/test_foo.py\n";
        let files = extract_test_files_from_patch(patch);
        assert_eq!(files, vec!["tests/test_foo.py"]);
    }

    #[test]
    fn test_extract_test_files_empty_patch() {
        assert!(extract_test_files_from_patch("").is_empty());
        assert!(extract_test_files_from_patch("no diff headers here").is_empty());
    }

    #[test]
    fn test_extract_test_ids_from_metadata() {
        let task = BenchTask {
            id: "t1".to_string(),
            prompt: String::new(),
            context: None,
            resources: vec![],
            tags: vec![],
            expected_turns: None,
            timeout: None,
            metadata: serde_json::json!({
                "fail_to_pass": "[\"test_a\", \"test_b\"]",
                "pass_to_pass": "[\"test_c\"]",
            }),
        };

        let (f2p, p2p) = extract_test_ids(&task);
        assert_eq!(f2p, vec!["test_a", "test_b"]);
        assert_eq!(p2p, vec!["test_c"]);
    }

    #[test]
    fn test_extract_test_ids_fallback_to_patch() {
        let task = BenchTask {
            id: "t2".to_string(),
            prompt: String::new(),
            context: None,
            resources: vec![],
            tags: vec![],
            expected_turns: None,
            timeout: None,
            metadata: serde_json::json!({
                "test_patch": "diff --git a/tests/t.py b/tests/t.py\n--- a/tests/t.py\n+++ b/tests/t.py\n@@ -1 +1 @@\n-old\n+new\n",
            }),
        };

        let (f2p, p2p) = extract_test_ids(&task);
        assert_eq!(f2p, vec!["tests/t.py"]);
        assert!(p2p.is_empty());
    }

    #[test]
    fn test_extract_test_ids_none_available() {
        let task = BenchTask {
            id: "t3".to_string(),
            prompt: String::new(),
            context: None,
            resources: vec![],
            tags: vec![],
            expected_turns: None,
            timeout: None,
            metadata: serde_json::json!({}),
        };

        let (f2p, p2p) = extract_test_ids(&task);
        assert!(f2p.is_empty());
        assert!(p2p.is_empty());
    }

    #[test]
    fn test_convert_to_django_test_id_pytest_format() {
        assert_eq!(
            convert_to_django_test_id("tests/admin_views/tests.py::AdminViewTest::test_foo"),
            "admin_views.tests.AdminViewTest.test_foo"
        );
    }

    #[test]
    fn test_convert_to_django_test_id_already_django() {
        assert_eq!(
            convert_to_django_test_id("admin_views.tests.AdminViewTest.test_foo"),
            "admin_views.tests.AdminViewTest.test_foo"
        );
    }

    #[test]
    fn test_convert_to_django_test_id_class_only() {
        assert_eq!(
            convert_to_django_test_id("tests/auth/test_models.py::TestModel"),
            "auth.test_models.TestModel"
        );
    }

    #[test]
    fn test_convert_to_django_test_id_no_tests_prefix() {
        assert_eq!(
            convert_to_django_test_id("auth/test_models.py::TestModel::test_x"),
            "auth.test_models.TestModel.test_x"
        );
    }

    #[test]
    fn test_build_test_command_pytest() {
        let cmd = build_test_command(
            "pytest --no-header -rN",
            &["tests/test_foo.py::TestBar::test_baz".to_string()],
            "some/repo",
        );
        assert_eq!(
            cmd,
            "pytest --no-header -rN tests/test_foo.py::TestBar::test_baz"
        );
    }

    #[test]
    fn test_build_test_command_django_converts_ids() {
        let cmd = build_test_command(
            "./tests/runtests.py --settings=test_sqlite --parallel 1",
            &["tests/admin_views/tests.py::AdminViewTest::test_foo".to_string()],
            "django/django",
        );
        assert_eq!(
            cmd,
            "./tests/runtests.py --settings=test_sqlite --parallel 1 admin_views.tests.AdminViewTest.test_foo"
        );
    }

    #[test]
    fn test_build_test_command_empty_ids() {
        let cmd = build_test_command("pytest", &[], "some/repo");
        assert_eq!(cmd, "pytest");
    }

    #[test]
    fn test_build_test_command_multiple_ids() {
        let cmd = build_test_command(
            "pytest",
            &["test_a".to_string(), "test_b".to_string()],
            "some/repo",
        );
        assert_eq!(cmd, "pytest test_a test_b");
    }

    #[test]
    fn test_build_test_command_escapes_special_chars() {
        let cmd = build_test_command(
            "pytest",
            &[
                "test_foo[True]".to_string(),
                "test_bar[\"hasattr(sys,".to_string(),
                "test_baz[*1".to_string(),
            ],
            "some/repo",
        );
        assert_eq!(
            cmd,
            "pytest 'test_foo[True]' 'test_bar[\"hasattr(sys,' 'test_baz[*1'"
        );
    }

    #[test]
    fn test_shell_escape() {
        // No special chars -> no quoting
        assert_eq!(shell_escape("test_foo"), "test_foo");
        assert_eq!(
            shell_escape("tests/test_a.py::TestB::test_c"),
            "tests/test_a.py::TestB::test_c"
        );
        // Special chars -> single-quoted
        assert_eq!(shell_escape("test[True]"), "'test[True]'");
        assert_eq!(shell_escape("test[*1"), "'test[*1'");
        // Single quotes inside -> escaped with '\''
        assert_eq!(shell_escape("test['x']"), r#"'test['\''x'\'']'"#);
        // Empty string
        assert_eq!(shell_escape(""), "''");
    }

    #[test]
    fn test_default_test_cmd_django() {
        assert!(default_test_cmd("django/django").contains("runtests.py"));
    }

    #[test]
    fn test_default_test_cmd_sympy() {
        assert!(default_test_cmd("sympy/sympy").contains("bin/test"));
    }

    #[test]
    fn test_default_test_cmd_fallback() {
        assert!(default_test_cmd("random/repo").contains("pytest"));
        assert!(default_test_cmd("scikit-learn/scikit-learn").contains("pytest"));
    }

    #[test]
    fn test_truncate_output_short() {
        let result = truncate_output("short stdout", "short stderr", 1000);
        assert!(result.contains("short stdout"));
        assert!(result.contains("short stderr"));
        assert!(!result.contains("truncated"));
    }

    #[test]
    fn test_truncate_output_long() {
        let long_stdout = "x".repeat(2000);
        let result = truncate_output(&long_stdout, "err", 500);
        assert!(result.contains("truncated"));
        // Should be roughly max_len plus the "... (truncated)\n" prefix
        assert!(result.len() < 600);
    }

    #[test]
    fn test_truncate_output_zero_max() {
        let result = truncate_output("stuff", "more stuff", 0);
        assert_eq!(result, "... (truncated)");
    }

    // ── Integration test: teardown captures diff ────────────────────────

    /// Helper to run a git command in a directory, asserting success.
    async fn git(dir: &Path, args: &[&str]) -> std::process::Output {
        let output = tokio::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    #[tokio::test]
    async fn test_teardown_captures_diff() {
        let workspace = tempfile::tempdir().unwrap();
        let task_dir = workspace.path().join("test-task-1");
        std::fs::create_dir_all(&task_dir).unwrap();

        // Initialize a git repo with an initial commit.
        // Disable gpg signing in case the system config enables it.
        git(&task_dir, &["init"]).await;
        git(&task_dir, &["config", "user.email", "test@test.com"]).await;
        git(&task_dir, &["config", "user.name", "Test"]).await;
        git(&task_dir, &["config", "commit.gpgsign", "false"]).await;

        std::fs::write(task_dir.join("hello.py"), "print('hello')\n").unwrap();
        git(&task_dir, &["add", "-A"]).await;
        git(&task_dir, &["commit", "-m", "initial"]).await;

        // Simulate agent making changes
        std::fs::write(task_dir.join("hello.py"), "print('hello world')\n").unwrap();
        std::fs::write(task_dir.join("new_file.py"), "# new file\n").unwrap();

        // Sanity check: verify git sees changes before teardown
        let status_out = git(&task_dir, &["status", "--porcelain"]).await;
        let status_str = String::from_utf8_lossy(&status_out.stdout);
        assert!(
            !status_str.trim().is_empty(),
            "git should see changes before teardown"
        );

        // Create suite and a dummy task
        let dataset = workspace.path().join("dummy.jsonl");
        std::fs::write(&dataset, "").unwrap();
        let suite = SweBenchSuite::new(&dataset, workspace.path(), false);

        let task = BenchTask {
            id: "test-task-1".to_string(),
            prompt: String::new(),
            context: None,
            resources: vec![],
            tags: vec![],
            expected_turns: None,
            timeout: None,
            metadata: serde_json::json!({}),
        };

        // Run teardown
        suite.teardown_task(&task).await.unwrap();

        // Verify patch was captured
        let patch_path = suite.agent_patch_path("test-task-1");
        assert!(patch_path.exists(), "patch file should exist");
        let patch = std::fs::read_to_string(&patch_path).unwrap();
        assert!(
            patch.contains("hello world"),
            "patch should contain the modification, got:\n{patch}"
        );
        assert!(
            patch.contains("new_file.py"),
            "patch should contain the new file, got:\n{patch}"
        );

        // Verify repo was cleaned up
        let status = tokio::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&task_dir)
            .output()
            .await
            .unwrap();
        let porcelain = String::from_utf8_lossy(&status.stdout);
        assert!(
            porcelain.trim().is_empty(),
            "repo should be clean after teardown, got: {porcelain}"
        );
    }

    // ── Docker helper unit tests ─────────────────────────────────────────

    #[test]
    fn test_docker_image_tag() {
        assert_eq!(
            docker_image_tag("pytest-dev/pytest", "69356d20c3abcdef1234"),
            "swebench-pytest-dev-pytest-69356d20c3"
        );
        assert_eq!(
            docker_image_tag("django/django", "abc123def456"),
            "swebench-django-django-abc123def4"
        );
    }

    #[test]
    fn test_docker_image_tag_short_commit() {
        assert_eq!(docker_image_tag("org/repo", "abc"), "swebench-org-repo-abc");
        assert_eq!(docker_image_tag("org/repo", ""), "swebench-org-repo-");
    }

    #[test]
    fn test_python_version_for_repo() {
        assert_eq!(
            python_version_for_repo("django/django", Some("4.2")),
            "3.11"
        );
        assert_eq!(
            python_version_for_repo("django/django", Some("5.0")),
            "3.11"
        );
        assert_eq!(python_version_for_repo("django/django", Some("3.2")), "3.9");
        assert_eq!(python_version_for_repo("django/django", None), "3.9");
        assert_eq!(python_version_for_repo("pytest-dev/pytest", None), "3.9");
        assert_eq!(python_version_for_repo("unknown/repo", None), "3.9");
    }

    #[test]
    fn test_read_exit_code_valid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exit"), "0\n").unwrap();
        assert_eq!(read_exit_code(dir.path(), "exit"), Some(0));
    }

    #[test]
    fn test_read_exit_code_nonzero() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exit"), "1").unwrap();
        assert_eq!(read_exit_code(dir.path(), "exit"), Some(1));
    }

    #[test]
    fn test_read_exit_code_missing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_exit_code(dir.path(), "nonexistent"), None);
    }

    #[test]
    fn test_read_exit_code_garbage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("exit"), "not a number").unwrap();
        assert_eq!(read_exit_code(dir.path(), "exit"), None);
    }

    // ── Docker integration tests (require Docker) ────────────────────────

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn test_docker_image_build() {
        let workspace = tempfile::tempdir().unwrap();
        let task_dir = workspace.path().join("test-repo");
        std::fs::create_dir_all(&task_dir).unwrap();

        // Create a minimal Python project with a git repo
        git(&task_dir, &["init"]).await;
        git(&task_dir, &["config", "user.email", "test@test.com"]).await;
        git(&task_dir, &["config", "user.name", "Test"]).await;
        git(&task_dir, &["config", "commit.gpgsign", "false"]).await;

        std::fs::write(
            task_dir.join("setup.py"),
            "from setuptools import setup\nsetup(name='testpkg', version='0.1')\n",
        )
        .unwrap();
        std::fs::write(task_dir.join("testpkg.py"), "def hello(): return 42\n").unwrap();
        git(&task_dir, &["add", "-A"]).await;
        git(&task_dir, &["commit", "-m", "initial"]).await;

        let tag = "swebench-test-build-integration";

        // Clean up any previous test image
        let _ = tokio::process::Command::new("docker")
            .args(["rmi", "-f", tag])
            .output()
            .await;

        // Build
        build_docker_image(&task_dir, tag, "3.9", "test-build")
            .await
            .unwrap();

        // Verify it exists
        assert!(docker_image_exists(tag).await);

        // Clean up
        let _ = tokio::process::Command::new("docker")
            .args(["rmi", "-f", tag])
            .output()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn test_docker_scoring_end_to_end() {
        let workspace = tempfile::tempdir().unwrap();
        let task_dir = workspace.path().join("test-score-repo");
        std::fs::create_dir_all(&task_dir).unwrap();

        // Create a minimal Python project: a file with a bug
        git(&task_dir, &["init"]).await;
        git(&task_dir, &["config", "user.email", "test@test.com"]).await;
        git(&task_dir, &["config", "user.name", "Test"]).await;
        git(&task_dir, &["config", "commit.gpgsign", "false"]).await;

        std::fs::write(
            task_dir.join("setup.py"),
            "from setuptools import setup\nsetup(name='testpkg', version='0.1')\n",
        )
        .unwrap();
        // Buggy: returns 41 instead of 42
        std::fs::write(task_dir.join("testpkg.py"), "def answer(): return 41\n").unwrap();
        std::fs::write(
            task_dir.join("test_pkg.py"),
            "from testpkg import answer\ndef test_answer(): assert answer() == 42\n",
        )
        .unwrap();
        git(&task_dir, &["add", "-A"]).await;
        git(&task_dir, &["commit", "-m", "initial with bug"]).await;

        // Get the base commit
        let log = git(&task_dir, &["rev-parse", "HEAD"]).await;
        let base_commit = String::from_utf8_lossy(&log.stdout).trim().to_string();

        let tag = "swebench-test-score-integration";

        // Clean up any previous test image
        let _ = tokio::process::Command::new("docker")
            .args(["rmi", "-f", tag])
            .output()
            .await;

        // Build the image
        build_docker_image(&task_dir, tag, "3.9", "test-score")
            .await
            .unwrap();

        // Agent patch: fix the bug (41 -> 42)
        let agent_patch = "\
diff --git a/testpkg.py b/testpkg.py
index 1234567..abcdefg 100644
--- a/testpkg.py
+++ b/testpkg.py
@@ -1 +1 @@
-def answer(): return 41
+def answer(): return 42
";

        // No separate test patch needed (tests already exist in base)
        let test_patch = "";

        let task = BenchTask {
            id: "test-score".to_string(),
            prompt: String::new(),
            context: None,
            resources: vec![],
            tags: vec![],
            expected_turns: None,
            timeout: None,
            metadata: serde_json::json!({
                "repo": "test/repo",
                "base_commit": base_commit,
                "fail_to_pass": "[\"test_pkg.py::test_answer\"]",
                "pass_to_pass": "[]",
            }),
        };

        let result = score_docker(
            &task,
            agent_patch,
            test_patch,
            tag,
            &base_commit,
            "test/repo",
        )
        .await;

        // Clean up
        let _ = tokio::process::Command::new("docker")
            .args(["rmi", "-f", tag])
            .output()
            .await;

        // The test_patch is empty, but the tests already exist at base commit.
        // The entrypoint will apply the empty test patch (no-op) and run the F2P tests.
        // With the agent fix applied, test_answer should pass.
        let score = result.unwrap();
        assert_eq!(
            score.value, 1.0,
            "expected pass but got: {:?}",
            score.details
        );
    }

    /// Full integration test: score the gold patch for pytest-dev__pytest-8906 using Docker.
    ///
    /// This is the exact scenario that failed locally with 13/84 P2P tests regressing
    /// due to `PluginValidationError` from the host's incompatible pluggy version.
    /// In Docker, the correct pluggy version is installed via `pip install -e .`, so
    /// all P2P tests should pass.
    ///
    /// Loads test data from the actual dataset file to exercise the real data path
    /// (including truncated P2P test IDs that need `resolve_test_ids_docker()`).
    ///
    /// Requires: Docker, network access (to clone pytest repo on first run),
    /// `benchmarks/data/swe-bench-lite.jsonl` must exist.
    #[tokio::test]
    #[ignore = "requires Docker + network, clones pytest repo"]
    async fn test_docker_score_pytest_8906_gold_patch() {
        // Load the real task from the dataset
        let dataset_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/swe-bench-lite.jsonl");
        if !dataset_path.exists() {
            panic!("dataset not found at {}", dataset_path.display());
        }

        let workspace = tempfile::tempdir().unwrap();
        let ws_path = workspace.path();

        let suite = SweBenchSuite::new(&dataset_path, ws_path, true);
        let all_tasks = suite.load_tasks().await.unwrap();
        let task = all_tasks
            .iter()
            .find(|t| t.id == "pytest-dev__pytest-8906")
            .expect("pytest-dev__pytest-8906 not found in dataset");

        // Setup: clone repo, checkout base commit, build Docker image
        suite.setup_task(task).await.unwrap();

        // Write the gold patch (from the dataset's `patch` field) as the agent's output
        let gold_patch = task
            .metadata
            .get("gold_patch")
            .and_then(|v| v.as_str())
            .expect("gold_patch missing from metadata");
        let patches_dir = suite.patches_dir();
        std::fs::create_dir_all(&patches_dir).unwrap();
        std::fs::write(suite.agent_patch_path(&task.id), gold_patch).unwrap();

        // Score with Docker
        let submission = TaskSubmission {
            response: String::new(),
            conversation: vec![],
            tool_calls: vec![],
            error: None,
        };

        let score = suite.score(task, &submission).await.unwrap();

        assert_eq!(
            score.value, 1.0,
            "gold patch should pass both F2P and P2P in Docker. Details: {:?}",
            score.details,
        );
    }
}
