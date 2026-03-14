//! Regression tests for shell command risk-level classification (issue #172).
//!
//! These tests cover two bugs fixed in the shell risk PR:
//!
//! 1. **Redirect bypass** -- Low-risk commands that contain shell redirections
//!    (`>`, `>>`) must return `UnlessAutoApproved`, not `Never`.  Before the
//!    fix, `Low` mapped to `Never`, which would let `echo secret > /etc/passwd`
//!    bypass approval entirely.
//!
//! 2. **git push classification** -- `git push` must be classified with
//!    `UnlessAutoApproved` approval (Medium risk), not `Always` (High). Force
//!    variants (`--force`, `-f`) must require `Always` (High risk).

use ironclaw::tools::{ApprovalRequirement, ToolRegistry};

async fn get_shell_tool() -> std::sync::Arc<dyn ironclaw::tools::Tool> {
    let registry = ToolRegistry::new();
    registry.register_builtin_tools();
    let tools = registry.all().await;
    tools
        .into_iter()
        .find(|t| t.name() == "shell")
        .expect("shell tool should be registered")
}

// ---------------------------------------------------------------------------
// Redirect bypass regression (Low-risk + redirect → UnlessAutoApproved, not Never)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn low_risk_command_with_redirect_is_unless_auto_approved() {
    let tool = get_shell_tool().await;
    let cases = [
        "echo secret_data > /etc/passwd",
        "cat /etc/shadow > /tmp/out",
        "date > /tmp/timestamp",
    ];
    for cmd in &cases {
        let params = serde_json::json!({ "command": cmd });
        let approval = tool.requires_approval(&params);
        assert_eq!(
            approval,
            ApprovalRequirement::UnlessAutoApproved,
            "command `{cmd}` should be UnlessAutoApproved (not Never), got {approval:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// git push classification regression (Medium risk → UnlessAutoApproved)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn git_push_is_unless_auto_approved() {
    let tool = get_shell_tool().await;
    for cmd in &[
        "git push",
        "git push origin main",
        "git push upstream feature/foo",
    ] {
        let params = serde_json::json!({ "command": cmd });
        let approval = tool.requires_approval(&params);
        assert_eq!(
            approval,
            ApprovalRequirement::UnlessAutoApproved,
            "command `{cmd}` should be UnlessAutoApproved, got {approval:?}"
        );
    }
}

#[tokio::test]
async fn git_push_force_requires_always_approval() {
    let tool = get_shell_tool().await;
    for cmd in &["git push --force", "git push -f", "git push --force-with-lease"] {
        let params = serde_json::json!({ "command": cmd });
        let approval = tool.requires_approval(&params);
        assert_eq!(
            approval,
            ApprovalRequirement::Always,
            "force-push `{cmd}` should require Always approval, got {approval:?}"
        );
    }
}
