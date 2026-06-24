//! Real-path workspace E2E for the GitHub issue workflow.
//!
//! These tests drive the PRODUCTION [`RuntimeWorkflowWorkspaceManager`] (the
//! composition-owned backend that shells out to real `git clone` / checkout /
//! commit / push and a real verify host-process) over a LOCAL BARE REPO served
//! through a `file://` clone URL. No network, no GitHub, no `FakeWorkspace
//! Manager`.
//!
//! Why this tier exists: the hermetic stage/poller tests use
//! `FakeWorkspaceManager` / `PublishingWorkspaceManager` with a hardcoded
//! `default_branch: "main"` and a fabricated `has_changes: true`. That fixture
//! shape hid a whole class of bugs the live runs then tripped:
//!
//!   1. EMPTY `default_branch` from a live GitHub payload made `git clone
//!      --branch ""` fail with "Remote branch  not found". The real manager
//!      must clone the remote's default HEAD and resolve the concrete branch
//!      afterwards.
//!   2. An empty base branch turned the publish commits-ahead check into
//!      `..HEAD` (counts zero), so publish wrongly SKIPPED the push. The real
//!      manager must resolve the concrete base from `origin/HEAD`.
//!   3. A non-zero verify exit had to surface as `Ok(passed: false)` — a policy
//!      decision, not an `Err`; a missing/zero command must report
//!      `ran: false` (skip the gate), not block the run.
//!
//! The tests are hermetic: a temp bare repo seeded via a temp working clone
//! (`tempfile`, never a hardcoded `/tmp`), and they early-return (skip) when
//! `git` is not on `PATH` so the suite stays green in environments without it.
#![cfg(all(feature = "github-issue-workflow-beta", feature = "test-support"))]

mod github_issue_workflow_workspace_e2e {
    use std::path::Path;
    use std::process::Command;

    use chrono::Utc;
    use ironclaw_github_issue_workflow::{
        GithubIssueRef, GithubIssueWorkflowRunId, GithubIssueWorkspaceSessionId,
        PrepareWorkflowWorkspaceRequest, PublishWorkflowWorkspaceRequest,
        VerifyWorkflowWorkspaceRequest, WorkflowVerificationCommand,
    };
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
    use ironclaw_reborn_composition::test_support::runtime_workflow_workspace_manager_for_test;

    /// Run a git command in `dir`, asserting success. A hermetic, isolated git
    /// environment: no system/global config is consulted (`GIT_CONFIG_NOSYSTEM`
    /// plus an empty `HOME`/`XDG_CONFIG_HOME`), terminal prompts are disabled,
    /// and author/committer identity is fixed so commits succeed without ambient
    /// user config. Mirrors the in-module bare-repo test helper.
    fn git(args: &[&str], dir: &Path, home: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    /// Whether `git` is available on `PATH`. The tests skip (early-return with
    /// an `eprintln`) when it is absent so the suite stays green in minimal
    /// environments; the production code path requires real git.
    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    /// Build a bare remote at `<root>/<owner>/<repo>.git` seeded with a single
    /// commit on `base_branch`, served through the `file://<root>` clone base.
    /// Returns the `file://` clone base URL the workspace manager points at.
    ///
    /// `base_branch` is made the remote default via `symbolic-ref HEAD` (so a
    /// clone-without-branch and `origin/HEAD` both resolve to it without relying
    /// on `init.defaultBranch`). Callers pass a non-"main" branch (e.g. "trunk"
    /// or "develop") to model the repos whose default is NOT "main".
    fn seed_bare_repo(
        root: &Path,
        home: &Path,
        owner: &str,
        repo: &str,
        base_branch: &str,
    ) -> String {
        let bare = root.join(owner).join(format!("{repo}.git"));
        std::fs::create_dir_all(&bare).expect("bare parent");
        git(&["init", "--bare", bare.to_str().unwrap()], root, home);

        let seed = root.join(format!("seed-{owner}-{repo}"));
        git(
            &["clone", bare.to_str().unwrap(), seed.to_str().unwrap()],
            root,
            home,
        );
        git(&["checkout", "-b", base_branch], &seed, home);
        std::fs::write(seed.join("README.md"), "fixture\n").expect("seed file");
        git(&["add", "."], &seed, home);
        git(&["commit", "-m", "seed"], &seed, home);
        git(&["push", "-u", "origin", base_branch], &seed, home);
        // Make `base_branch` the remote default so a clone-without-branch and
        // origin/HEAD resolve to it.
        git(
            &["symbolic-ref", "HEAD", &format!("refs/heads/{base_branch}")],
            &bare,
            home,
        );

        format!("file://{}", root.display())
    }

    /// Locate the single prepared checkout under the workspace storage root.
    /// The production manager writes it to
    /// `<storage>/github-issue-workspaces/<workspace_session_id>/`; this resolves
    /// it from the session id without depending on a crate-private path helper.
    fn checkout_path(
        storage_root: &Path,
        session_id: &GithubIssueWorkspaceSessionId,
    ) -> std::path::PathBuf {
        storage_root
            .join("github-issue-workspaces")
            .join(session_id.as_str())
    }

    fn issue(owner: &str, repo: &str, number: u64, default_branch: &str) -> GithubIssueRef {
        GithubIssueRef {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
            node_id: None,
            url: format!("https://example.invalid/{owner}/{repo}/issues/{number}"),
            default_branch: default_branch.to_string(),
        }
    }

    fn run_id(label: &str) -> GithubIssueWorkflowRunId {
        GithubIssueWorkflowRunId::from_trusted(label.to_string()).expect("workflow run id")
    }

    fn tenant() -> TenantId {
        TenantId::new("t").expect("tenant")
    }

    fn user() -> UserId {
        UserId::new("u").expect("user")
    }

    fn agent() -> Option<AgentId> {
        Some(AgentId::new("a").expect("agent"))
    }

    fn project() -> Option<ProjectId> {
        Some(ProjectId::new("p").expect("project"))
    }

    fn prepare_request(
        run: &GithubIssueWorkflowRunId,
        issue: &GithubIssueRef,
        base_branch: &str,
    ) -> PrepareWorkflowWorkspaceRequest {
        PrepareWorkflowWorkspaceRequest {
            tenant_id: tenant(),
            creator_user_id: user(),
            agent_id: agent(),
            project_id: project(),
            workflow_run_id: run.clone(),
            issue: issue.clone(),
            base_branch: base_branch.to_string(),
            requested_at: Utc::now(),
        }
    }

    fn publish_request(
        run: &GithubIssueWorkflowRunId,
        issue: &GithubIssueRef,
        session_id: &GithubIssueWorkspaceSessionId,
        base_branch: &str,
    ) -> PublishWorkflowWorkspaceRequest {
        PublishWorkflowWorkspaceRequest {
            tenant_id: tenant(),
            creator_user_id: user(),
            agent_id: agent(),
            project_id: project(),
            workflow_run_id: run.clone(),
            issue: issue.clone(),
            workspace_session_id: session_id.clone(),
            base_branch: base_branch.to_string(),
            commit_message: "ironclaw: apply fix".to_string(),
            requested_at: Utc::now(),
        }
    }

    fn verify_request(
        run: &GithubIssueWorkflowRunId,
        issue: &GithubIssueRef,
        session_id: &GithubIssueWorkspaceSessionId,
        command: Option<WorkflowVerificationCommand>,
    ) -> VerifyWorkflowWorkspaceRequest {
        VerifyWorkflowWorkspaceRequest {
            tenant_id: tenant(),
            creator_user_id: user(),
            agent_id: agent(),
            project_id: project(),
            workflow_run_id: run.clone(),
            issue: issue.clone(),
            workspace_session_id: session_id.clone(),
            command,
            requested_at: Utc::now(),
        }
    }

    /// List the remote's `refs/heads/*` from the bare repo via plumbing.
    fn remote_branch_refs(bare: &Path) -> String {
        let output = Command::new("git")
            .args([
                "--git-dir",
                bare.to_str().unwrap(),
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/",
            ])
            .output()
            .expect("list remote refs");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// The tree the published commit actually contains (HEAD of the checkout).
    fn committed_tree(checkout: &Path) -> String {
        let output = Command::new("git")
            .args([
                "-C",
                checkout.to_str().unwrap(),
                "ls-tree",
                "-r",
                "--name-only",
                "HEAD",
            ])
            .output()
            .expect("ls-tree");
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    /// Guards bug class (1): a real `git clone` over a `file://` bare repo on an
    /// explicit non-"main" base branch ("develop") produces a real checkout —
    /// the working branch is created and the base SHA is resolved from the
    /// actual remote HEAD (not fabricated as the fake did).
    #[tokio::test]
    async fn prepare_real_checkout_on_non_main_base_branch() {
        if !git_available() {
            eprintln!("skipping: `git` is not on PATH");
            return;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let home = tempfile::tempdir().expect("home");
        let owner = "ironclaw-e2e";
        let repo = "develop-fixture";
        let clone_base = seed_bare_repo(root.path(), home.path(), owner, repo, "develop");

        let storage = root.path().join("storage");
        std::fs::create_dir_all(&storage).expect("storage");
        let manager = runtime_workflow_workspace_manager_for_test(storage.clone(), clone_base);

        let run = run_id("workflow-run-develop-base");
        // The live payload carries the real default branch here; assert the
        // manager honors an explicit non-"main" base (the fake hardcoded "main").
        let issue = issue(owner, repo, 11, "develop");
        let prepared = manager
            .prepare_workspace(prepare_request(&run, &issue, "develop"))
            .await
            .expect("prepare workspace over file:// bare repo");

        assert_eq!(
            prepared.session.base_branch, "develop",
            "explicit non-main base must be honored, not coerced to \"main\""
        );
        assert!(
            prepared.session.base_sha.is_some(),
            "a real clone must resolve a concrete base SHA"
        );
        assert!(
            prepared
                .session
                .working_branch
                .contains(&format!("issue-{}", issue.number)),
            "working branch must be derived from the issue: {}",
            prepared.session.working_branch
        );

        // The checkout actually exists on disk and is the seeded working branch.
        let checkout = checkout_path(&storage, &prepared.session.workspace_session_id);
        assert!(
            checkout.join("README.md").exists(),
            "the seeded file must be present in the real checkout"
        );
        let head_branch = Command::new("git")
            .args([
                "-C",
                checkout.to_str().unwrap(),
                "rev-parse",
                "--abbrev-ref",
                "HEAD",
            ])
            .output()
            .expect("rev-parse HEAD branch");
        let head_branch = String::from_utf8_lossy(&head_branch.stdout);
        assert_eq!(
            head_branch.trim(),
            prepared.session.working_branch,
            "checkout HEAD must be on the manager-reported working branch"
        );
    }

    /// Guards bug classes (1) + (2): when the live GitHub payload omits the
    /// default branch, `base_branch` arrives EMPTY. Prepare must clone the
    /// remote default HEAD and resolve the concrete branch (here "trunk", not
    /// "" and not "main"); publish must then resolve the concrete base from
    /// `origin/HEAD`, detect the new commit, and ACTUALLY PUSH (the empty-base
    /// commits-ahead bug made it silently skip the push).
    #[tokio::test]
    async fn prepare_then_publish_with_empty_base_resolves_default_and_pushes() {
        if !git_available() {
            eprintln!("skipping: `git` is not on PATH");
            return;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let home = tempfile::tempdir().expect("home");
        let owner = "ironclaw-e2e";
        let repo = "empty-base-fixture";
        let clone_base = seed_bare_repo(root.path(), home.path(), owner, repo, "trunk");
        let bare = root.path().join(owner).join(format!("{repo}.git"));

        let storage = root.path().join("storage");
        std::fs::create_dir_all(&storage).expect("storage");
        let manager = runtime_workflow_workspace_manager_for_test(storage.clone(), clone_base);

        let run = run_id("workflow-run-empty-base");
        // The empty default_branch the live GitHub payload yields.
        let issue = issue(owner, repo, 7, "");
        let prepared = manager
            .prepare_workspace(prepare_request(&run, &issue, ""))
            .await
            .expect("prepare workspace with empty base");
        assert_eq!(
            prepared.session.base_branch, "trunk",
            "empty base must resolve to the remote default branch, not \"\" or \"main\""
        );

        let session_id = prepared.session.workspace_session_id.clone();
        let checkout = checkout_path(&storage, &session_id);
        std::fs::write(checkout.join("fix.txt"), "the fix\n").expect("write change");

        let published = manager
            .publish_workspace(publish_request(&run, &issue, &session_id, ""))
            .await
            .expect("publish workspace");
        assert!(
            published.has_changes,
            "publish must detect the new commit and push, not silently skip on empty base"
        );
        assert_eq!(
            published.base_branch, "trunk",
            "publish must resolve the concrete base from origin/HEAD, not leave it empty"
        );

        let refs = remote_branch_refs(&bare);
        assert!(
            refs.contains(&published.working_branch),
            "pushed working branch {} not found in remote refs:\n{refs}",
            published.working_branch
        );
    }

    /// Guards the no-op-publish branch of bug class (2): with NO edit in the
    /// checkout, publish must report `has_changes: false` and must NOT push an
    /// empty branch to the remote (no working-branch ref appears).
    #[tokio::test]
    async fn publish_without_edit_reports_no_changes_and_does_not_push() {
        if !git_available() {
            eprintln!("skipping: `git` is not on PATH");
            return;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let home = tempfile::tempdir().expect("home");
        let owner = "ironclaw-e2e";
        let repo = "no-edit-fixture";
        let clone_base = seed_bare_repo(root.path(), home.path(), owner, repo, "trunk");
        let bare = root.path().join(owner).join(format!("{repo}.git"));

        let storage = root.path().join("storage");
        std::fs::create_dir_all(&storage).expect("storage");
        let manager = runtime_workflow_workspace_manager_for_test(storage.clone(), clone_base);

        let run = run_id("workflow-run-no-edit");
        let issue = issue(owner, repo, 8, "");
        let prepared = manager
            .prepare_workspace(prepare_request(&run, &issue, ""))
            .await
            .expect("prepare workspace");
        let session_id = prepared.session.workspace_session_id.clone();

        // No edit made to the checkout before publishing.
        let published = manager
            .publish_workspace(publish_request(&run, &issue, &session_id, ""))
            .await
            .expect("publish workspace with no changes");
        assert!(
            !published.has_changes,
            "publish with no commits beyond base must report has_changes: false"
        );

        let refs = remote_branch_refs(&bare);
        assert!(
            !refs.contains(&published.working_branch),
            "no empty working branch must be pushed; refs:\n{refs}"
        );
        // The seeded base branch is still the only head on the remote.
        assert!(
            refs.contains("refs/heads/trunk"),
            "the seeded base branch must remain on the remote:\n{refs}"
        );
    }

    /// Guards bug class (3): the real verify host-process reports exit codes
    /// faithfully. An explicit passing command (`true`) -> ran + passed; an
    /// explicit failing command (`false`) -> ran + NOT passed AND is `Ok`
    /// (a policy decision, never an `Err`); no command + no detectable runner
    /// (README-only repo) -> the gate is skipped (`ran: false`, `passed: true`).
    #[tokio::test]
    async fn verify_reports_exit_codes_faithfully_for_pass_fail_and_skip() {
        if !git_available() {
            eprintln!("skipping: `git` is not on PATH");
            return;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let home = tempfile::tempdir().expect("home");
        let owner = "ironclaw-e2e";
        let repo = "verify-fixture";
        let clone_base = seed_bare_repo(root.path(), home.path(), owner, repo, "trunk");

        let storage = root.path().join("storage");
        std::fs::create_dir_all(&storage).expect("storage");
        let manager = runtime_workflow_workspace_manager_for_test(storage.clone(), clone_base);

        let run = run_id("workflow-run-verify");
        let issue = issue(owner, repo, 9, "");
        let prepared = manager
            .prepare_workspace(prepare_request(&run, &issue, ""))
            .await
            .expect("prepare workspace");
        let session_id = prepared.session.workspace_session_id.clone();

        // Passing command (exit 0) -> ran + passed.
        let passing = manager
            .verify_workspace(verify_request(
                &run,
                &issue,
                &session_id,
                Some(WorkflowVerificationCommand {
                    program: "true".to_string(),
                    args: Vec::new(),
                    timeout_secs: 30,
                }),
            ))
            .await
            .expect("verify `true`");
        assert!(
            passing.ran && passing.passed,
            "exit-0 command must report ran + passed"
        );
        assert_eq!(passing.exit_code, Some(0), "exit code 0 must be reported");

        // Failing command (exit non-zero) -> ran + NOT passed, surfaced as Ok.
        let failing = manager
            .verify_workspace(verify_request(
                &run,
                &issue,
                &session_id,
                Some(WorkflowVerificationCommand {
                    program: "false".to_string(),
                    args: Vec::new(),
                    timeout_secs: 30,
                }),
            ))
            .await
            .expect("a failing verify command is Ok(passed: false), not Err");
        assert!(
            failing.ran && !failing.passed,
            "a non-zero exit must report ran + NOT passed (policy decision, not Err)"
        );
        assert_ne!(
            failing.exit_code,
            Some(0),
            "a failing command must not report exit code 0"
        );

        // No command + no detectable runner (README-only) -> skip the gate.
        let skipped = manager
            .verify_workspace(verify_request(&run, &issue, &session_id, None))
            .await
            .expect("verify auto-detect");
        assert!(
            !skipped.ran && skipped.passed,
            "no detected runner must skip the gate (ran: false, passed: true)"
        );
    }

    /// Guards bug class (3) with a real test runner: a seeded shell script that
    /// the host-process runs argv-only. A passing script -> ran + passed; the
    /// same script flipped to fail -> ran + NOT passed (Ok, exit non-zero).
    /// Exercises the host-process exit handling on an actual repo file rather
    /// than only the `true`/`false` builtins.
    #[tokio::test]
    async fn verify_runs_seeded_script_and_reports_pass_then_fail() {
        if !git_available() {
            eprintln!("skipping: `git` is not on PATH");
            return;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let home = tempfile::tempdir().expect("home");
        let owner = "ironclaw-e2e";
        let repo = "script-fixture";
        let clone_base = seed_bare_repo(root.path(), home.path(), owner, repo, "trunk");

        let storage = root.path().join("storage");
        std::fs::create_dir_all(&storage).expect("storage");
        let manager = runtime_workflow_workspace_manager_for_test(storage.clone(), clone_base);

        let run = run_id("workflow-run-script");
        let issue = issue(owner, repo, 10, "");
        let prepared = manager
            .prepare_workspace(prepare_request(&run, &issue, ""))
            .await
            .expect("prepare workspace");
        let session_id = prepared.session.workspace_session_id.clone();
        let checkout = checkout_path(&storage, &session_id);

        // Seed a passing test script into the checkout and run it through the
        // verify host-process (sh <script>, argv-only — no shell string).
        std::fs::write(checkout.join("run_tests.sh"), "#!/bin/sh\nexit 0\n")
            .expect("write passing script");
        let passing = manager
            .verify_workspace(verify_request(
                &run,
                &issue,
                &session_id,
                Some(WorkflowVerificationCommand {
                    program: "sh".to_string(),
                    args: vec!["run_tests.sh".to_string()],
                    timeout_secs: 30,
                }),
            ))
            .await
            .expect("verify passing script");
        assert!(
            passing.ran && passing.passed,
            "the seeded passing script must report ran + passed"
        );

        // Flip the script to fail; the exit code must surface as Ok(passed:false).
        std::fs::write(checkout.join("run_tests.sh"), "#!/bin/sh\nexit 3\n")
            .expect("write failing script");
        let failing = manager
            .verify_workspace(verify_request(
                &run,
                &issue,
                &session_id,
                Some(WorkflowVerificationCommand {
                    program: "sh".to_string(),
                    args: vec!["run_tests.sh".to_string()],
                    timeout_secs: 30,
                }),
            ))
            .await
            .expect("a failing script is Ok(passed: false), not Err");
        assert!(
            failing.ran && !failing.passed,
            "the seeded failing script must report ran + NOT passed"
        );
        assert_eq!(
            failing.exit_code,
            Some(3),
            "the script's concrete non-zero exit code must be reported faithfully"
        );
    }

    /// Guards the publish artifact-exclusion path on the real backend: a
    /// `__pycache__/*.pyc` build artifact seeded into the checkout must be kept
    /// out of the published commit (via the workspace-local `.git/info/exclude`
    /// the manager seeds at prepare time), while the genuine fix IS committed.
    #[tokio::test]
    async fn publish_commits_fix_but_excludes_build_artifacts() {
        if !git_available() {
            eprintln!("skipping: `git` is not on PATH");
            return;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let home = tempfile::tempdir().expect("home");
        let owner = "ironclaw-e2e";
        let repo = "exclude-fixture";
        let clone_base = seed_bare_repo(root.path(), home.path(), owner, repo, "trunk");

        let storage = root.path().join("storage");
        std::fs::create_dir_all(&storage).expect("storage");
        let manager = runtime_workflow_workspace_manager_for_test(storage.clone(), clone_base);

        let run = run_id("workflow-run-exclude");
        let issue = issue(owner, repo, 12, "");
        let prepared = manager
            .prepare_workspace(prepare_request(&run, &issue, ""))
            .await
            .expect("prepare workspace");
        let session_id = prepared.session.workspace_session_id.clone();
        let checkout = checkout_path(&storage, &session_id);

        std::fs::write(checkout.join("fix.txt"), "the fix\n").expect("write change");
        std::fs::create_dir_all(checkout.join("__pycache__")).expect("pycache dir");
        std::fs::write(checkout.join("__pycache__").join("x.pyc"), b"\x00bytecode")
            .expect("write pyc");

        let published = manager
            .publish_workspace(publish_request(&run, &issue, &session_id, ""))
            .await
            .expect("publish workspace");
        assert!(published.has_changes, "the real fix must publish a change");

        let committed = committed_tree(&checkout);
        assert!(
            committed.contains("fix.txt"),
            "the fix must be committed:\n{committed}"
        );
        assert!(
            !committed.contains("__pycache__") && !committed.contains(".pyc"),
            "build artifacts must be excluded from the commit:\n{committed}"
        );
    }
}
