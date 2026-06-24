//! Local-dev filesystem workspace backend for the GitHub issue workflow.
//!
//! [`RuntimeWorkflowWorkspaceManager`] clones, branches, verifies, and publishes
//! the per-run workspace on the host filesystem, shelling out through the
//! hardened [`super::git_host`] primitives. Verification is host-process based
//! and therefore LOCAL-DEV ONLY (see `git_host`); production swaps a sandboxed
//! backend behind the same [`WorkflowWorkspaceManager`] trait.
//! [`UnconfiguredWorkflowWorkspaceManager`] fails closed when no backend is
//! wired.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_github_issue_workflow::{
    GithubIssueWorkflowError, GithubIssueWorkflowRunId, GithubIssueWorkspaceSession,
    GithubIssueWorkspaceSessionId, GithubRepositorySelector, PrepareWorkflowWorkspaceOutcome,
    PrepareWorkflowWorkspaceRequest, PublishWorkflowWorkspaceOutcome,
    PublishWorkflowWorkspaceRequest, VerifyWorkflowWorkspaceOutcome,
    VerifyWorkflowWorkspaceRequest, WorkflowWorkspaceManager, WorkflowWorkspaceRef,
};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use tracing::debug;

use super::git_host::{
    WorkflowGitRemoteConfig, run_workflow_git_command, run_workflow_git_command_with_config,
    run_workflow_host_process,
};

const GITHUB_ISSUE_WORKFLOW_WORKSPACES_STORAGE_DIR: &str = "github-issue-workspaces";
const GITHUB_ISSUE_WORKFLOW_WORKSPACES_VIRTUAL_ROOT: &str = "/projects/github-issue-workspaces";

pub(crate) fn runtime_workflow_workspace_manager(
    local_dev_storage_root: PathBuf,
) -> Arc<dyn WorkflowWorkspaceManager> {
    Arc::new(RuntimeWorkflowWorkspaceManager {
        local_dev_storage_root,
        git_remote: WorkflowGitRemoteConfig::local_dev_default(),
    })
}

/// Test-only constructor that points the REAL workspace manager at an arbitrary
/// `clone_base_url` (e.g. a `file://` bare repo) with no credential helper and a
/// fixed committer, so the production clone/publish/verify path can be exercised
/// hermetically over a local bare repo (no network, no GitHub). Mirrors the
/// in-module bare-repo tests; exposed through `crate::test_support` so the
/// composition crate's integration tests can drive the real backend.
#[cfg(any(test, feature = "test-support"))]
pub fn runtime_workflow_workspace_manager_for_test(
    local_dev_storage_root: PathBuf,
    clone_base_url: String,
) -> Arc<dyn WorkflowWorkspaceManager> {
    Arc::new(RuntimeWorkflowWorkspaceManager {
        local_dev_storage_root,
        git_remote: WorkflowGitRemoteConfig {
            config_args: Vec::new(),
            clone_base_url,
            committer_name: "IronClaw Bot".to_string(),
            committer_email: "bot@ironclaw.test".to_string(),
        },
    })
}

pub(super) struct RuntimeWorkflowWorkspaceManager {
    pub(super) local_dev_storage_root: PathBuf,
    pub(super) git_remote: WorkflowGitRemoteConfig,
}

#[async_trait]
impl WorkflowWorkspaceManager for RuntimeWorkflowWorkspaceManager {
    async fn prepare_workspace(
        &self,
        request: PrepareWorkflowWorkspaceRequest,
    ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        let workspace_session_id = GithubIssueWorkspaceSessionId::new();
        let repository =
            GithubRepositorySelector::new(request.issue.owner.clone(), request.issue.repo.clone())?;
        let clone_url =
            workflow_repository_clone_url(&repository, &self.git_remote.clone_base_url)?;
        let workspace_host_path =
            workflow_workspace_host_path(&self.local_dev_storage_root, &workspace_session_id);
        let workspaces_root =
            workspace_host_path
                .parent()
                .ok_or_else(|| GithubIssueWorkflowError::Repository {
                    reason: "workflow workspace path has no parent directory".to_string(),
                })?;
        tokio::fs::create_dir_all(workspaces_root)
            .await
            .map_err(|error| GithubIssueWorkflowError::Repository {
                reason: format!("failed to create workflow workspace root: {error}"),
            })?;
        let workspace_host_path_str =
            workspace_host_path
                .to_str()
                .ok_or_else(|| GithubIssueWorkflowError::Repository {
                    reason: "workflow workspace path is not valid UTF-8".to_string(),
                })?;
        // The live GitHub issue/search payloads do not carry the repository's
        // default branch, so `request.base_branch` is frequently empty. Passing
        // `--branch ""` to `git clone` fails with "Remote branch  not found".
        // When the base branch is unknown, clone the remote's default HEAD (which
        // is exactly the repository default branch) and resolve the concrete
        // branch name afterwards so the PR base ref is still correct.
        let base_branch_arg = request.base_branch.trim();
        let mut clone_args: Vec<&str> = vec!["clone", "--no-tags", "--depth", "1"];
        if !base_branch_arg.is_empty() {
            clone_args.push("--branch");
            clone_args.push(base_branch_arg);
        }
        clone_args.push(&clone_url);
        clone_args.push(workspace_host_path_str);
        run_workflow_git_command_with_config(None, &self.git_remote.config_args, &clone_args)
            .await?;
        // Keep build artifacts / caches out of the eventual `git add -A` at
        // publish time by seeding the workspace-local git excludes
        // (`.git/info/exclude`, NOT the repo-tracked `.gitignore`). The model's
        // shell/test runs commonly produce `__pycache__/*.pyc`, `node_modules/`,
        // `target/`, etc.; without this they get swept into the PR commit.
        // Best-effort: a failure here must not abort the clone.
        {
            use tokio::io::AsyncWriteExt;
            let exclude_path = workspace_host_path
                .join(".git")
                .join("info")
                .join("exclude");
            let exclude_body = "\n# ironclaw github issue workflow: keep build artifacts/caches out of the PR\n__pycache__/\n*.pyc\n*.pyo\n.pytest_cache/\n.mypy_cache/\n.ruff_cache/\nnode_modules/\ntarget/\n";
            match tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&exclude_path)
                .await
            {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(exclude_body.as_bytes()).await {
                        tracing::debug!(?error, "failed to seed workflow git excludes");
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        ?error,
                        "could not open workspace git exclude to seed artifact ignores"
                    );
                }
            }
        }
        // Resolve the concrete base branch when the request did not specify one.
        let base_branch = if base_branch_arg.is_empty() {
            run_workflow_git_command(
                Some(&workspace_host_path),
                &["rev-parse", "--abbrev-ref", "HEAD"],
            )
            .await?
            .trim()
            .to_string()
        } else {
            request.base_branch.clone()
        };
        let base_sha =
            run_workflow_git_command(Some(&workspace_host_path), &["rev-parse", "HEAD"]).await?;
        let working_branch = workflow_working_branch(&request.issue, &request.workflow_run_id);
        run_workflow_git_command(
            Some(&workspace_host_path),
            &["checkout", "-B", &working_branch],
        )
        .await?;
        let current_head_sha =
            run_workflow_git_command(Some(&workspace_host_path), &["rev-parse", "HEAD"]).await?;
        let workspace_ref = WorkflowWorkspaceRef {
            thread_id: None,
            workspace_session_id: Some(workspace_session_id.clone()),
            turn_run_id: None,
        };
        let mount_ref = ironclaw_github_issue_workflow::WorkflowWorkspaceMountRef {
            mount_id: workspace_session_id.as_str().to_string(),
            alias: crate::local_dev_mounts::WORKSPACE_ALIAS.to_string(),
        };
        tracing::debug!(
            workflow_run_id = %request.workflow_run_id,
            workspace_session_id = %workspace_session_id,
            owner = %request.issue.owner,
            repo = %request.issue.repo,
            base_branch = %base_branch,
            working_branch = %working_branch,
            base_sha = %short_sha(&base_sha),
            mount_alias = %mount_ref.alias,
            "prepared github issue workflow workspace clone"
        );
        Ok(PrepareWorkflowWorkspaceOutcome {
            session: GithubIssueWorkspaceSession {
                workspace_session_id,
                workflow_run_id: request.workflow_run_id.clone(),
                repository,
                base_branch,
                base_sha: Some(base_sha),
                working_branch,
                current_head_sha: Some(current_head_sha),
                workspace_ref,
                mount_ref,
                created_at: request.requested_at,
            },
        })
    }

    async fn publish_workspace(
        &self,
        request: PublishWorkflowWorkspaceRequest,
    ) -> Result<PublishWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        let repository =
            GithubRepositorySelector::new(request.issue.owner.clone(), request.issue.repo.clone())?;
        let clone_url =
            workflow_repository_clone_url(&repository, &self.git_remote.clone_base_url)?;
        let workspace_host_path = workflow_workspace_host_path(
            &self.local_dev_storage_root,
            &request.workspace_session_id,
        );
        if !tokio::fs::try_exists(&workspace_host_path)
            .await
            .unwrap_or(false)
        {
            return Err(GithubIssueWorkflowError::Repository {
                reason: "workflow workspace checkout is missing; cannot publish branch".to_string(),
            });
        }
        let working_branch = workflow_working_branch(&request.issue, &request.workflow_run_id);
        // The live GitHub issue payload does not carry the repository default
        // branch, so `request.base_branch` is frequently empty. An empty base
        // turns the commits-ahead check below into `..HEAD` (which counts zero),
        // making publish wrongly skip the push. Resolve the concrete base from
        // the checkout's remote default branch when none was supplied.
        let base_branch = if request.base_branch.trim().is_empty() {
            // prepare_workspace clones WITHOUT `--branch` when the base is empty,
            // which always sets origin/HEAD, so this resolves in practice. The
            // `HEAD~1` fallback is a documented last resort that keeps the
            // commits-ahead check meaningful (counts the single new commit) if
            // origin/HEAD is somehow unset; we log when it fires.
            let resolved = run_workflow_git_command(
                Some(&workspace_host_path),
                &["rev-parse", "--abbrev-ref", "origin/HEAD"],
            )
            .await
            .ok()
            .map(|head| head.trim().trim_start_matches("origin/").to_string())
            .filter(|branch| !branch.is_empty());
            match resolved {
                Some(branch) => branch,
                None => {
                    debug!(
                        "publish_workspace could not resolve origin/HEAD for an empty base branch; using HEAD~1 commits-ahead heuristic"
                    );
                    "HEAD~1".to_string()
                }
            }
        } else {
            request.base_branch.clone()
        };

        // Stage every change the implementation agent made in the checkout.
        run_workflow_git_command(Some(&workspace_host_path), &["add", "-A"]).await?;
        // Commit the staged changes if the working tree is dirty. (The agent may
        // also have committed itself; in that case there is nothing to stage and
        // this is a no-op.) `git status --porcelain` is empty iff the tree is
        // clean after staging.
        let porcelain =
            run_workflow_git_command(Some(&workspace_host_path), &["status", "--porcelain"])
                .await?;
        if !porcelain.trim().is_empty() {
            run_workflow_git_command(
                Some(&workspace_host_path),
                &[
                    "-c",
                    &format!("user.name={}", self.git_remote.committer_name),
                    "-c",
                    &format!("user.email={}", self.git_remote.committer_email),
                    "commit",
                    "--no-verify",
                    "-m",
                    &request.commit_message,
                ],
            )
            .await?;
        }

        // A draft PR needs at least one commit between base and the working
        // branch. Propagate a git failure here rather than collapsing it to "0":
        // an empty diff is already detected above by `git status --porcelain`
        // after `add -A`, so a `rev-list` error is a real fault (bad base ref,
        // corrupt repo) that must NOT be silently read as "no changes" and skip
        // the push (error-handling.md: no silent-ok swallow).
        let commits_ahead = run_workflow_git_command(
            Some(&workspace_host_path),
            &["rev-list", "--count", &format!("{base_branch}..HEAD")],
        )
        .await?;
        let has_changes = commits_ahead.trim() != "0";
        let head_sha =
            run_workflow_git_command(Some(&workspace_host_path), &["rev-parse", "HEAD"]).await?;

        if !has_changes {
            tracing::debug!(
                workflow_run_id = %request.workflow_run_id,
                working_branch = %working_branch,
                "workflow workspace has no commits beyond base; skipping push"
            );
            return Ok(PublishWorkflowWorkspaceOutcome {
                working_branch,
                base_branch,
                head_sha,
                has_changes: false,
            });
        }

        // Push the working branch to the remote using the configured credential
        // helper. The refspec pushes the current HEAD to the named branch so the
        // draft PR can reference it. `--force-with-lease` keeps a re-run safe
        // without clobbering unrelated remote history.
        run_workflow_git_command_with_config(
            Some(&workspace_host_path),
            &self.git_remote.config_args,
            &[
                "push",
                "--force-with-lease",
                &clone_url,
                &format!("HEAD:refs/heads/{working_branch}"),
            ],
        )
        .await?;
        tracing::debug!(
            workflow_run_id = %request.workflow_run_id,
            owner = %request.issue.owner,
            repo = %request.issue.repo,
            working_branch = %working_branch,
            head_sha = %short_sha(&head_sha),
            "pushed workflow workspace branch to remote"
        );

        Ok(PublishWorkflowWorkspaceOutcome {
            working_branch,
            base_branch,
            head_sha,
            has_changes: true,
        })
    }

    /// Run the repository's verification command in the prepared checkout. See
    /// `run_workflow_host_process` — this is LOCAL-DEV ONLY (host process running
    /// arbitrary repo code); production must verify inside the Docker sandbox.
    async fn verify_workspace(
        &self,
        request: VerifyWorkflowWorkspaceRequest,
    ) -> Result<VerifyWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        let workspace_host_path = workflow_workspace_host_path(
            &self.local_dev_storage_root,
            &request.workspace_session_id,
        );
        if !tokio::fs::try_exists(&workspace_host_path)
            .await
            .unwrap_or(false)
        {
            return Err(GithubIssueWorkflowError::Repository {
                reason: "workflow workspace checkout is missing; cannot verify".to_string(),
            });
        }
        // Host-authored command if supplied, else auto-detect a runner from the
        // checkout. No command and no detected runner => skip the gate.
        let command = match &request.command {
            Some(command) => Some((
                command.program.clone(),
                command.args.clone(),
                command.timeout_secs,
            )),
            None => detect_workflow_verification_command(&workspace_host_path).await,
        };
        let Some((program, args, timeout_secs)) = command else {
            tracing::debug!(
                workflow_run_id = %request.workflow_run_id,
                "no workflow verification command configured or detected; skipping gate"
            );
            return Ok(VerifyWorkflowWorkspaceOutcome {
                ran: false,
                passed: true,
                exit_code: None,
                command_label: String::new(),
                stdout_tail: String::new(),
                stderr_tail: String::new(),
            });
        };
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let command_label = if args.is_empty() {
            program.clone()
        } else {
            format!("{program} {}", args.join(" "))
        };
        let output = run_workflow_host_process(
            Some(&workspace_host_path),
            &program,
            &arg_refs,
            timeout_secs,
        )
        .await?;
        if output.not_found {
            // The verification runner is not installed in this environment.
            // Treat as "no verification available" (skip) rather than a failed
            // test suite — retrying or blocking would be wrong for a missing tool.
            tracing::debug!(
                workflow_run_id = %request.workflow_run_id,
                command = %command_label,
                "workflow verification runner not found; skipping gate"
            );
            return Ok(VerifyWorkflowWorkspaceOutcome {
                ran: false,
                passed: true,
                exit_code: None,
                command_label,
                stdout_tail: String::new(),
                stderr_tail: output.stderr_tail,
            });
        }
        tracing::debug!(
            workflow_run_id = %request.workflow_run_id,
            command = %command_label,
            passed = output.success,
            exit_code = ?output.exit_code,
            "ran workflow workspace verification command"
        );
        Ok(VerifyWorkflowWorkspaceOutcome {
            ran: true,
            passed: output.success,
            exit_code: output.exit_code,
            command_label,
            stdout_tail: output.stdout_tail,
            stderr_tail: output.stderr_tail,
        })
    }
}

pub(super) fn workflow_workspace_host_path(
    local_dev_storage_root: &Path,
    workspace_session_id: &GithubIssueWorkspaceSessionId,
) -> PathBuf {
    local_dev_storage_root
        .join(GITHUB_ISSUE_WORKFLOW_WORKSPACES_STORAGE_DIR)
        .join(workspace_session_id.as_str())
}

fn workflow_workspace_virtual_root(
    workspace_session_id: &GithubIssueWorkspaceSessionId,
) -> Result<VirtualPath, GithubIssueWorkflowError> {
    VirtualPath::new(format!(
        "{GITHUB_ISSUE_WORKFLOW_WORKSPACES_VIRTUAL_ROOT}/{}",
        workspace_session_id.as_str()
    ))
    .map_err(|error| GithubIssueWorkflowError::Policy {
        reason: format!("invalid workflow workspace virtual root: {error}"),
    })
}

pub(super) fn workflow_workspace_mount_view(
    workspace_session_id: &GithubIssueWorkspaceSessionId,
    alias: &str,
) -> Result<MountView, GithubIssueWorkflowError> {
    MountView::new(vec![MountGrant::new(
        MountAlias::new(alias.to_string()).map_err(|error| GithubIssueWorkflowError::Policy {
            reason: format!("invalid workflow workspace mount alias: {error}"),
        })?,
        workflow_workspace_virtual_root(workspace_session_id)?,
        // The implementation stage agent edits, deletes, and runs shell/build
        // commands inside the cloned `/workspace`. It therefore needs full
        // workspace authority including `execute` (without it the shell handler
        // rejects a `/workspace` workdir) and `delete` (for apply_patch removals
        // and file deletions during the fix).
        MountPermissions::read_write_list_delete_execute(),
    )])
    .map_err(|error| GithubIssueWorkflowError::Policy {
        reason: format!("invalid workflow workspace mount view: {error}"),
    })
}

fn workflow_repository_clone_url(
    repository: &GithubRepositorySelector,
    clone_base_url: &str,
) -> Result<String, GithubIssueWorkflowError> {
    validate_github_url_component("repository owner", &repository.owner)?;
    validate_github_url_component("repository name", &repository.repo)?;
    Ok(format!(
        "{}/{}/{}.git",
        clone_base_url.trim_end_matches('/'),
        repository.owner,
        repository.repo
    ))
}

fn validate_github_url_component(label: &str, value: &str) -> Result<(), GithubIssueWorkflowError> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: format!("{label} is not a safe GitHub URL component"),
        });
    }
    if !value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(GithubIssueWorkflowError::InvalidConfig {
            reason: format!("{label} contains characters that are not safe in a GitHub URL"),
        });
    }
    Ok(())
}

const WORKFLOW_VERIFY_DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Auto-detect a test/verification command from the workspace checkout when no
/// host command is configured. A small, documented heuristic; returns the argv
/// (program, args, timeout_secs) or `None` when no recognizable runner is found.
async fn detect_workflow_verification_command(
    workspace: &Path,
) -> Option<(String, Vec<String>, u64)> {
    async fn has(workspace: &Path, name: &str) -> bool {
        tokio::fs::try_exists(workspace.join(name))
            .await
            .unwrap_or(false)
    }
    if has(workspace, "Cargo.toml").await {
        return Some((
            "cargo".to_string(),
            vec!["test".to_string(), "--quiet".to_string()],
            WORKFLOW_VERIFY_DEFAULT_TIMEOUT_SECS,
        ));
    }
    if has(workspace, "package.json").await {
        return Some((
            "npm".to_string(),
            vec!["test".to_string(), "--silent".to_string()],
            WORKFLOW_VERIFY_DEFAULT_TIMEOUT_SECS,
        ));
    }
    let python = has(workspace, "pyproject.toml").await
        || has(workspace, "setup.py").await
        || has(workspace, "conftest.py").await
        || workspace_has_pytest_file(workspace).await;
    if python {
        return Some((
            "python3".to_string(),
            vec!["-m".to_string(), "pytest".to_string(), "-q".to_string()],
            WORKFLOW_VERIFY_DEFAULT_TIMEOUT_SECS,
        ));
    }
    None
}

async fn workspace_has_pytest_file(workspace: &Path) -> bool {
    let Ok(mut entries) = tokio::fs::read_dir(workspace).await else {
        return false;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(name) = entry.file_name().to_str()
            && name.starts_with("test_")
            && name.ends_with(".py")
        {
            return true;
        }
    }
    false
}

fn workflow_working_branch(
    issue: &ironclaw_github_issue_workflow::GithubIssueRef,
    workflow_run_id: &GithubIssueWorkflowRunId,
) -> String {
    let owner = git_branch_component(&issue.owner);
    let repo = git_branch_component(&issue.repo);
    let short_run_id: String = workflow_run_id.as_str().chars().take(12).collect();
    format!(
        "ironclaw/github-bug/{owner}-{repo}-issue-{}-{short_run_id}",
        issue.number
    )
}

/// Short (first 12 chars) form of a git SHA for non-sensitive audit logging.
fn short_sha(sha: &str) -> String {
    sha.chars().take(12).collect()
}

pub(super) fn git_branch_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|character| match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => character,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if sanitized.is_empty() {
        "repo".to_string()
    } else {
        sanitized
    }
}

pub(super) struct UnconfiguredWorkflowWorkspaceManager;

#[async_trait]
impl WorkflowWorkspaceManager for UnconfiguredWorkflowWorkspaceManager {
    async fn prepare_workspace(
        &self,
        _request: PrepareWorkflowWorkspaceRequest,
    ) -> Result<PrepareWorkflowWorkspaceOutcome, GithubIssueWorkflowError> {
        Err(GithubIssueWorkflowError::PolicyDenied {
            reason: "GitHub issue workflow workspace backend is not configured".to_string(),
        })
    }
}
