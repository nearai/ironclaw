//! Regression and unit tests for shell command risk-level classification
//! (issue #172, PR #368).
//!
//! These tests live here (instead of inline in `src/tools/builtin/shell.rs`)
//! because the project's no-panics CI check scans `src/**/*.rs` for
//! `assert_eq!` / `assert_ne!` / `.unwrap()` in added lines.  All assertions
//! on the public `ShellTool` API belong here.
//!
//! All tests access the shell tool through the public `ToolRegistry` +
//! `Tool` trait surface (`risk_level_for`, `requires_approval`).
//!
//! ## What is tested
//!
//! 1. **Risk level tiers** (`High`, `Medium`, `Low`) for representative commands.
//! 2. **Word-boundary matching** — commands whose names are substrings of other
//!    words must not be misclassified.
//! 3. **Pipeline aggregation** — the whole pipeline takes the maximum risk of
//!    its segments.
//! 4. **Redirect bypass regression** — Low-risk commands with shell redirections
//!    must return `UnlessAutoApproved`, not `Never`.
//! 5. **`git push` regression** — non-force push is explicitly `Medium`; force
//!    variants remain `High`.
//! 6. **`risk_level_for` trait method** — delegates to classify_command_risk.

use ironclaw::tools::{ApprovalRequirement, RiskLevel, Tool, ToolRegistry};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helper: obtain a `ShellTool` from the registry
// ---------------------------------------------------------------------------

async fn shell_tool() -> Arc<dyn Tool> {
    let registry = ToolRegistry::new();
    registry.register_builtin_tools();
    registry.register_dev_tools();
    registry
        .all()
        .await
        .into_iter()
        .find(|t| t.name() == "shell")
        .expect("shell tool must be registered")
}

fn risk(tool: &Arc<dyn Tool>, cmd: &str) -> RiskLevel {
    tool.risk_level_for(&serde_json::json!({ "command": cmd }))
}

fn approval(tool: &Arc<dyn Tool>, cmd: &str) -> ApprovalRequirement {
    tool.requires_approval(&serde_json::json!({ "command": cmd }))
}

// ---------------------------------------------------------------------------
// 1. Risk level tiers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn high_risk_commands() {
    let tool = shell_tool().await;
    let cmds = [
        "rm -rf /tmp/stuff",
        "git push --force origin main",
        "git reset --hard HEAD~5",
        "docker rm container_name",
        "kill -9 12345",
        "DROP TABLE users;",
        "sudo apt install something",
    ];
    for cmd in &cmds {
        assert_eq!(
            risk(&tool, cmd),
            RiskLevel::High,
            "command `{cmd}` should be High risk"
        );
    }
}

#[tokio::test]
async fn low_risk_commands() {
    let tool = shell_tool().await;
    let cmds = [
        "ls -la",
        "cat file.txt",
        "grep foo bar.txt",
        "git status",
        "git log --oneline",
        "echo hello",
        "cargo check",
    ];
    for cmd in &cmds {
        assert_eq!(
            risk(&tool, cmd),
            RiskLevel::Low,
            "command `{cmd}` should be Low risk"
        );
    }
}

#[tokio::test]
async fn medium_risk_commands() {
    let tool = shell_tool().await;
    let cmds = [
        "cargo build",
        "cargo test",
        "npm test",
        "yarn test",
        "git commit -m 'foo'",
        "mkdir /tmp/dir",
        "npm install lodash",
        "git push origin feature-branch",
        "my-custom-tool --flag",
        "sed 's/foo/bar/g' file.txt",
        "sed -i 's/foo/bar/' file.txt",
        "awk '{print $1}' file.txt",
        "find . -name '*.rs'",
        "find . -delete",
    ];
    for cmd in &cmds {
        assert_eq!(
            risk(&tool, cmd),
            RiskLevel::Medium,
            "command `{cmd}` should be Medium risk"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Word-boundary matching (no false positives for substrings)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn word_boundary_no_false_positives() {
    let tool = shell_tool().await;
    // "lsblk" must NOT match "ls" (Low-risk prefix)
    assert_eq!(risk(&tool, "lsblk"), RiskLevel::Medium);
    // "makeself" must NOT match "make"
    assert_eq!(risk(&tool, "makeself output.run"), RiskLevel::Medium);
    // "git statusbar" must NOT match "git status"
    assert_eq!(risk(&tool, "git statusbar"), RiskLevel::Medium);
    // Commands with High-risk names as substrings must not be tagged High
    assert_eq!(risk(&tool, "makeshutdownscript --help"), RiskLevel::Medium);
    assert_eq!(risk(&tool, "nftables-config"), RiskLevel::Medium);
    assert_eq!(risk(&tool, "passwdqc-check"), RiskLevel::Medium);
}

#[tokio::test]
async fn word_boundary_correct_positive_matches() {
    let tool = shell_tool().await;
    assert_eq!(risk(&tool, "ls -la"), RiskLevel::Low);
    assert_eq!(risk(&tool, "make install"), RiskLevel::Medium);
    assert_eq!(risk(&tool, "git status"), RiskLevel::Low);
}

// ---------------------------------------------------------------------------
// 3. Pipeline aggregation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pipeline_takes_max_risk() {
    let tool = shell_tool().await;
    // High-risk segment → whole pipeline is High
    assert_eq!(risk(&tool, "ls /tmp | rm -rf /tmp/stuff"), RiskLevel::High);
    // All-low pipeline stays Low
    assert_eq!(risk(&tool, "ls -la | grep foo"), RiskLevel::Low);
    // Low + Medium → max is Medium
    assert_eq!(risk(&tool, "echo hello | cargo build"), RiskLevel::Medium);
    // Unknown command in pipeline → Medium (safe default)
    assert_eq!(
        risk(&tool, "cat file.txt | my-custom-tool"),
        RiskLevel::Medium
    );
}

// ---------------------------------------------------------------------------
// 4. Redirect bypass regression (Low → UnlessAutoApproved, not Never)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn low_risk_command_with_redirect_is_unless_auto_approved() {
    let tool = shell_tool().await;
    let cases = [
        "echo secret_data > /etc/passwd",
        "cat /etc/shadow > /tmp/exfil.txt",
        "printf '%s' value > /tmp/leak",
        "ls -la >> /tmp/log.txt",
    ];
    for cmd in &cases {
        let result = approval(&tool, cmd);
        assert_eq!(
            result,
            ApprovalRequirement::UnlessAutoApproved,
            "command `{cmd}` must be UnlessAutoApproved (not Never), got {result:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. git push regressions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn git_push_classifies_as_medium_risk() {
    let tool = shell_tool().await;
    let cmds = [
        "git push",
        "git push origin main",
        "git push --set-upstream origin feature",
        "git push upstream feature/foo",
    ];
    for cmd in &cmds {
        assert_eq!(risk(&tool, cmd), RiskLevel::Medium, "command `{cmd}`");
    }
}

#[tokio::test]
async fn git_push_force_remains_high_risk() {
    let tool = shell_tool().await;
    let cmds = [
        "git push --force",
        "git push -f",
        "git push --force-with-lease",
        "git push --force origin main",
        "git push -f origin main",
    ];
    for cmd in &cmds {
        assert_eq!(risk(&tool, cmd), RiskLevel::High, "command `{cmd}`");
    }
}

#[tokio::test]
async fn git_push_non_force_is_unless_auto_approved() {
    let tool = shell_tool().await;
    let cmds = [
        "git push",
        "git push origin main",
        "git push upstream feature/foo",
    ];
    for cmd in &cmds {
        let result = approval(&tool, cmd);
        assert_eq!(
            result,
            ApprovalRequirement::UnlessAutoApproved,
            "command `{cmd}` should be UnlessAutoApproved, got {result:?}"
        );
    }
}

#[tokio::test]
async fn git_push_force_requires_always_approval() {
    let tool = shell_tool().await;
    let cmds = [
        "git push --force",
        "git push -f",
        "git push --force-with-lease",
    ];
    for cmd in &cmds {
        let result = approval(&tool, cmd);
        assert_eq!(
            result,
            ApprovalRequirement::Always,
            "force-push `{cmd}` should require Always approval, got {result:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 6. risk_level_for trait method
// ---------------------------------------------------------------------------

#[tokio::test]
async fn risk_level_for_via_tool_trait() {
    let tool = shell_tool().await;
    assert_eq!(risk(&tool, "ls -la"), RiskLevel::Low);
    assert_eq!(risk(&tool, "cargo build"), RiskLevel::Medium);
    assert_eq!(risk(&tool, "rm -rf /tmp"), RiskLevel::High);
    // Missing params → Medium (safe default)
    assert_eq!(
        tool.risk_level_for(&serde_json::json!({})),
        RiskLevel::Medium
    );
}
