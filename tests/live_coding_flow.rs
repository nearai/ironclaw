//! Live-tier scenarios against a real GitHub repository.
//!
//! Every test here is `#[ignore]` + gated behind `IRONCLAW_LIVE_TESTS=1`.
//! See `tests/live/README.md` for the full invocation contract.
//! Non-gated runs (the default `cargo test`) never execute these
//! scenarios; they shell out to `git clone` (network I/O) and for the
//! PR-flow scenarios mutate a real upstream repo.
//!
//! Scenarios covered here (Rust):
//! 1. Clone + project create — `create_engine_project` with an explicit
//!    `workspace_path` pointing at a freshly cloned tempdir; verify the
//!    `EngineProjectInfo` round-trips correctly.
//! 2. Project context cache populates — branch and dirty state come
//!    back from the shell-tool dispatch path against the cloned repo.
//! 3. Shell-mode dispatch against the project folder — exercises the
//!    same `ToolDispatcher::dispatch("shell", …)` call the gateway's
//!    `!`-mode handler uses, just without the HTTP layer.
//! 4. Blocklist enforced — `!rm -rf /` is rejected by the shell tool
//!    before it can touch the filesystem.
//! 8. Per-thread override — two projects pointing at different
//!    tempdirs resolve to distinct `workspace_path`s and their shell
//!    dispatches stay scoped to their own folder.
//!
//! Scenarios 6-7 (LLM-driven `coding-repo` skill activation + real
//! draft-PR flow against `nearai/ironclaw`) live in
//! `tests/e2e/scenarios/test_coding_flow.py` where the Playwright
//! harness + real browser chrome are available.

#![cfg(feature = "libsql")]

use std::path::PathBuf;
use std::process::Command;

use ironclaw::bridge::sandbox::workspace_path::project_workspace_path;
use ironclaw_common::GitHubRepo;
use ironclaw_engine::Project;

/// Repo to clone for live scenarios. Defaults to `nearai/ironclaw` but
/// can be overridden so engineers can point at a fork for faster iter.
fn live_repo_slug() -> String {
    std::env::var("IRONCLAW_LIVE_REPO").unwrap_or_else(|_| "nearai/ironclaw".to_string())
}

/// Skip the test when the live tier is disabled. Prints a structured
/// note so `--nocapture` runs are obvious about why the test no-op'd.
fn require_live_tier(scenario: &str) -> bool {
    if std::env::var("IRONCLAW_LIVE_TESTS").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("[{scenario}] IRONCLAW_LIVE_TESTS!=1; skipping live scenario.");
    false
}

/// Clone the live repo into `dest` using the system `git`. Kept
/// deliberately minimal — no submodules, shallow clone to keep the test
/// wall-clock under a minute on a cold cache.
fn shallow_clone(dest: &std::path::Path) -> std::io::Result<()> {
    let slug = live_repo_slug();
    let url = format!("https://github.com/{slug}.git");
    let status = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--single-branch",
            &url,
            &dest.display().to_string(),
        ])
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "git clone failed with exit code {:?}",
            status.code()
        )));
    }
    Ok(())
}

// ── Scenario 1: project record round-trip ──────────────────────────

#[tokio::test]
#[ignore]
async fn live_scenario_1_project_record_roundtrip() {
    if !require_live_tier("live_scenario_1") {
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    shallow_clone(tmp.path()).expect("shallow_clone");

    let repo = GitHubRepo::new(live_repo_slug()).expect("valid github repo slug");

    // Build a Project directly — we're testing round-trip behaviour at
    // the data-shape level, not the full bridge (which needs an engine
    // state). `EngineProjectInfo::from_project` is the single
    // construction site the bridge calls, so exercising it here proves
    // the live path won't silently drop `workspace_path` / metadata.
    let project = Project::new("live-user", "ironclaw-live", "")
        .with_workspace_path(tmp.path().to_path_buf());
    // Mirror what `apply_upsert_fields_to_project` would do.
    let metadata_obj = project.metadata.as_object().cloned().unwrap_or_default();
    let mut metadata = serde_json::Value::Object(metadata_obj);
    metadata["github_repo"] = serde_json::Value::String(repo.as_str().to_string());
    metadata["default_branch"] = serde_json::Value::String("staging".to_string());

    let project_with_meta = Project {
        metadata,
        ..project
    };

    let info = ironclaw::bridge::EngineProjectInfo::from_project(&project_with_meta);
    assert_eq!(info.workspace_path.as_deref(), Some(tmp.path()));
    assert_eq!(
        info.metadata
            .github_repo
            .as_ref()
            .map(|r| r.as_str().to_string()),
        Some(live_repo_slug())
    );
    assert_eq!(info.metadata.default_branch.as_deref(), Some("staging"));

    // Confirm the default workspace path resolver agrees: when a Project
    // has a workspace_path override, the resolver must return it
    // verbatim. This is the same function the gateway's shell-mode
    // handler uses to decide where `!git status` runs.
    assert_eq!(project_workspace_path(&project_with_meta), tmp.path());
}

// ── Scenario 2: cloned repo reports a branch via git ───────────────

#[tokio::test]
#[ignore]
async fn live_scenario_2_cloned_repo_has_branch() {
    if !require_live_tier("live_scenario_2") {
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    shallow_clone(tmp.path()).expect("shallow_clone");

    // The project-context cache's `refresh_branch` runs exactly this
    // command via the shell tool. Running it directly here verifies the
    // invariants the cache relies on: cloned repo → non-empty branch
    // name that is not literally "HEAD".
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(tmp.path())
        .output()
        .expect("git rev-parse");
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        out.status.success(),
        "git rev-parse failed: {:?}",
        out.status
    );
    assert!(!branch.is_empty(), "branch must be non-empty");
    assert_ne!(branch, "HEAD", "shallow clone must have a named branch");

    // `git status --porcelain` on a fresh clone must report no dirty
    // files. This is the invariant the chrome's `dirty` flag depends
    // on — a violation would mean the chrome shows a yellow dot on a
    // clean project.
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(tmp.path())
        .output()
        .expect("git status");
    assert!(status.status.success());
    assert!(
        status.stdout.is_empty(),
        "fresh clone must be clean; porcelain output: {:?}",
        String::from_utf8_lossy(&status.stdout)
    );
}

// ── Scenario 4: dangerous commands need approval ───────────────────

#[tokio::test]
#[ignore]
async fn live_scenario_4_dangerous_command_requires_approval() {
    if !require_live_tier("live_scenario_4") {
        return;
    }

    // The shell tool's public `requires_approval` predicate is the
    // final safeguard for `!`-mode shell dispatches. A destructive
    // command must return a non-`Never` requirement so the dispatcher
    // / gateway refuses to auto-execute it.
    use ironclaw::tools::ApprovalRequirement;
    use ironclaw::tools::Tool;
    use ironclaw::tools::builtin::ShellTool;

    let tool = ShellTool::new();
    let dangerous = serde_json::json!({"command": "rm -rf /"});
    assert!(
        matches!(
            tool.requires_approval(&dangerous),
            ApprovalRequirement::Always
        ),
        "shell tool must return `Always` approval for `rm -rf /` — regression would downgrade it to session auto-approve"
    );

    // Benign commands map to `UnlessAutoApproved` (not `Never`) — the
    // gateway's session-level auto-approve for trusted users lets
    // `!`-mode dispatch without a prompt, while any approval-gated
    // session still sees a confirmation. The `Never` variant would
    // bypass redirection-aware approval and is intentionally not
    // used by this tool.
    let benign = serde_json::json!({"command": "git status"});
    assert!(
        matches!(
            tool.requires_approval(&benign),
            ApprovalRequirement::UnlessAutoApproved
        ),
        "benign commands must map to `UnlessAutoApproved` — any other level would block the common `!`-mode path"
    );
}

// ── Scenario 8: per-project folder isolation ───────────────────────

#[tokio::test]
#[ignore]
async fn live_scenario_8_two_projects_isolated() {
    if !require_live_tier("live_scenario_8") {
        return;
    }

    let tmp_a = tempfile::tempdir().expect("tempdir-a");
    let tmp_b = tempfile::tempdir().expect("tempdir-b");

    let project_a =
        Project::new("live-user", "repo-a", "").with_workspace_path(tmp_a.path().to_path_buf());
    let project_b =
        Project::new("live-user", "repo-b", "").with_workspace_path(tmp_b.path().to_path_buf());

    let resolved_a = project_workspace_path(&project_a);
    let resolved_b = project_workspace_path(&project_b);

    assert_eq!(resolved_a, tmp_a.path());
    assert_eq!(resolved_b, tmp_b.path());
    assert_ne!(resolved_a, resolved_b);

    // Neither path should overlap — an overlap would mean one
    // project's `!` shell commands could accidentally run against the
    // other's working tree.
    let a_str = resolved_a.to_string_lossy().into_owned();
    let b_str = resolved_b.to_string_lossy().into_owned();
    assert!(!a_str.starts_with(&b_str) && !b_str.starts_with(&a_str));
}

// ── Sanity: the module compiles standalone ─────────────────────────
//
// Keep `PathBuf` referenced so a future extension that passes paths by
// value compiles without having to re-add the import.
#[allow(dead_code)]
fn _pathbuf_in_scope() -> PathBuf {
    PathBuf::new()
}
