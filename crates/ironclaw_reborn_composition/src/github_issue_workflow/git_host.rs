//! Host process + git command primitives for the GitHub issue workflow — the
//! security boundary for everything this workflow shells out to.
//!
//! # Security invariants (pure-move isolation of the boundary, not new behavior)
//!
//! - **No token in argv.** Credential material is routed through `git -c
//!   credential.helper=…` config args / `GIT_ASKPASS` / a credential helper —
//!   never inlined into a clone/push URL or `args`. The argv is therefore safe
//!   to `debug!`-log, and no secret reaches the persisted remote config.
//! - **`GIT_CONFIG_NOSYSTEM=1`** ignores the machine-wide `/etc` gitconfig so a
//!   system-level alias / `insteadOf` / credential rule cannot silently rewrite
//!   the workflow's git commands.
//! - **`GIT_TERMINAL_PROMPT=0` + `GIT_SSH_COMMAND` `BatchMode=yes`** keep every
//!   command headless: a missing credential or unknown host-key never blocks on
//!   an interactive prompt (which would hang the poller until the timeout).
//! - **`kill_on_drop(true)` + a bounded `tokio::time::timeout`** guarantee a
//!   stuck child process cannot outlive its future or wedge the poller.
//! - **Host-process verification is LOCAL-DEV ONLY.** `run_workflow_host_process`
//!   executes arbitrary repository-authored code (`conftest.py`, `build.rs`, npm
//!   scripts, …) as a HOST process with the operator's privileges. Production
//!   MUST run verification inside the per-project Docker sandbox; the
//!   `WorkflowWorkspaceManager` trait seam allows swapping a sandboxed backend.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use ironclaw_github_issue_workflow::GithubIssueWorkflowError;
use tokio::process::Command;

const WORKFLOW_WORKSPACE_GIT_TIMEOUT: Duration = Duration::from_secs(300);

/// How the workflow authenticates git remote operations (clone of private
/// repos, push of the working branch) and identifies its commits.
#[derive(Debug, Clone)]
pub(super) struct WorkflowGitRemoteConfig {
    /// `git -c <value>` overrides applied to remote operations — typically a
    /// credential helper. Values are passed as process args (never inlined into
    /// a URL) so no token appears in argv or the persisted remote config.
    pub(super) config_args: Vec<String>,
    /// Base the repository clone URL is built from
    /// (`{clone_base_url}/{owner}/{repo}.git`). Defaults to `https://github.com`;
    /// overridable for GitHub Enterprise or hermetic `file://` tests.
    pub(super) clone_base_url: String,
    pub(super) committer_name: String,
    pub(super) committer_email: String,
}

impl WorkflowGitRemoteConfig {
    /// Local-dev default: authenticate via the `gh` CLI credential helper
    /// (clearing any inherited helper first) so clone/push use the operator's
    /// configured GitHub auth. Production should instead inject an
    /// account-scoped credential rather than relying on ambient `gh` auth.
    pub(super) fn local_dev_default() -> Self {
        Self {
            config_args: vec![
                "credential.helper=".to_string(),
                "credential.helper=!gh auth git-credential".to_string(),
            ],
            clone_base_url: "https://github.com".to_string(),
            committer_name: "IronClaw Bot".to_string(),
            committer_email: "ironclaw-bot@users.noreply.github.com".to_string(),
        }
    }
}

pub(super) async fn run_workflow_git_command(
    current_dir: Option<&Path>,
    args: &[&str],
) -> Result<String, GithubIssueWorkflowError> {
    run_workflow_git_command_with_config(current_dir, &[], args).await
}

/// Run a workflow git command with optional `-c key=value` config overrides
/// (e.g. a credential helper for remote operations). Config values are passed
/// as separate process args after a `-c` flag — never interpolated into a URL —
/// so credentials do not appear in argv logging or the remote config.
pub(super) async fn run_workflow_git_command_with_config(
    current_dir: Option<&Path>,
    config_args: &[String],
    args: &[&str],
) -> Result<String, GithubIssueWorkflowError> {
    let mut command = Command::new("git");
    for config in config_args {
        command.arg("-c").arg(config);
    }
    command.args(args).stdin(Stdio::null()).kill_on_drop(true);
    // Git hygiene for workflow-owned checkouts:
    // - GIT_TERMINAL_PROMPT=0 / GIT_SSH_COMMAND `BatchMode` never block on an
    //   interactive credential or host-key prompt (the workflow runs headless;
    //   a prompt would otherwise hang the poller until the timeout).
    // - GIT_CONFIG_NOSYSTEM=1 ignores the machine-wide /etc gitconfig so a
    //   system-level alias/insteadof/credential rule can't silently rewrite the
    //   workflow's commands.
    // The per-user credential helper IS still available so local-dev can push
    // using the operator's configured GitHub auth; production should instead
    // supply an explicit account-scoped credential (see publish step).
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    // The argv is safe to log: callers route credential material through env /
    // GIT_ASKPASS / a credential helper, never inline in `args` (see the push
    // step's authenticated-remote handling), so no token reaches this line.
    tracing::debug!(git_args = ?args, "running workflow git command");
    let output = tokio::time::timeout(WORKFLOW_WORKSPACE_GIT_TIMEOUT, command.output())
        .await
        .map_err(|_| GithubIssueWorkflowError::Repository {
            reason: "git command timed out while preparing workflow workspace".to_string(),
        })?
        .map_err(|error| GithubIssueWorkflowError::Repository {
            reason: format!("failed to run git while preparing workflow workspace: {error}"),
        })?;
    if !output.status.success() {
        return Err(GithubIssueWorkflowError::Repository {
            reason: format!(
                "git command failed while preparing workflow workspace: {}",
                workflow_command_stderr_summary(&output.stderr)
            ),
        });
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_string())
        .map_err(|error| GithubIssueWorkflowError::Repository {
            reason: format!("git output was not valid UTF-8: {error}"),
        })
}

const WORKFLOW_VERIFY_OUTPUT_TAIL_CHARS: usize = 2048;

pub(super) struct WorkflowProcessOutput {
    pub(super) success: bool,
    pub(super) exit_code: Option<i32>,
    /// True when the program itself could not be found (not installed). Distinct
    /// from "ran and exited non-zero" so callers can skip rather than retry/block.
    pub(super) not_found: bool,
    pub(super) stdout_tail: String,
    pub(super) stderr_tail: String,
}

pub(super) fn tail_chars(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= max {
        trimmed.to_string()
    } else {
        chars[chars.len() - max..].iter().collect()
    }
}

/// Run a host process (argv-only — NEVER a shell string) in `current_dir` with
/// the same headless hardening the git runner uses (stdin null, kill_on_drop,
/// timeout, GIT_* prompt suppression — harmless for non-git programs). Returns
/// the raw outcome (exit status + bounded output tails) WITHOUT erroring on a
/// non-zero exit, so the verification gate can distinguish "ran and failed" (a
/// policy decision) from a spawn/timeout fault (an `Err`).
///
/// LOCAL-DEV ONLY when used for verification: this executes arbitrary
/// repository-authored code (conftest.py, build.rs, npm scripts, …) as a HOST
/// process with the operator's privileges. Production MUST run verification
/// inside the per-project Docker sandbox; the `WorkflowWorkspaceManager` trait
/// seam allows swapping a sandboxed backend.
pub(super) async fn run_workflow_host_process(
    current_dir: Option<&Path>,
    program: &str,
    args: &[&str],
    timeout_secs: u64,
) -> Result<WorkflowProcessOutput, GithubIssueWorkflowError> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).kill_on_drop(true);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    tracing::debug!(program = %program, args = ?args, "running workflow workspace command");
    let timeout = Duration::from_secs(timeout_secs.clamp(1, 1800));
    let output = match tokio::time::timeout(timeout, command.output()).await {
        Err(_) => {
            return Err(GithubIssueWorkflowError::Repository {
                reason: format!("workflow verification command `{program}` timed out"),
            });
        }
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkflowProcessOutput {
                success: false,
                exit_code: None,
                not_found: true,
                stdout_tail: String::new(),
                stderr_tail: format!("verification program `{program}` was not found"),
            });
        }
        Ok(Err(error)) => {
            return Err(GithubIssueWorkflowError::Repository {
                reason: format!("failed to run workflow verification command `{program}`: {error}"),
            });
        }
        Ok(Ok(output)) => output,
    };
    Ok(WorkflowProcessOutput {
        success: output.status.success(),
        exit_code: output.status.code(),
        not_found: false,
        stdout_tail: tail_chars(
            &String::from_utf8_lossy(&output.stdout),
            WORKFLOW_VERIFY_OUTPUT_TAIL_CHARS,
        ),
        stderr_tail: workflow_command_stderr_summary(&output.stderr),
    })
}

pub(super) fn workflow_command_stderr_summary(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "<no stderr>".to_string()
    } else {
        trimmed.chars().take(1000).collect()
    }
}
