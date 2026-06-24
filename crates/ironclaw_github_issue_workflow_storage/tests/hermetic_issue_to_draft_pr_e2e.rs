//! Hermetic, no-Docker end-to-end test: drive the REAL
//! `GithubIssueWorkflowPoller::tick_once()` over a durable libSQL repository
//! through the full path — discover → claim → Triage → Planning →
//! Implementation → PrSynthesis → workspace publish → draft PR — then simulate
//! a process restart (`RepositoryCase::reopen()` opens a fresh handle over the
//! same on-disk libSQL file) and replay, asserting NO duplicate draft PR.
//!
//! Stage results are reported BETWEEN ticks via
//! `ironclaw_github_issue_workflow::testing::complete_active_stage` (see that
//! module's docs for why synchronous in-`submit_stage_turn` injection would
//! corrupt the run version mid-tick).

#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw_github_issue_workflow::testing::{self, RecordingGithubPort, TestPollerPorts};
use ironclaw_github_issue_workflow::{
    GithubIssueRef, GithubIssueWorkflowConfig, GithubIssueWorkflowMode, GithubIssueWorkflowPoller,
    GithubIssueWorkflowRepository,
};

mod support;

use support::RepositoryCase;

const POLICY_VERSION: &str = "github-bug-workflow-v1";
const MAX_TICKS: usize = 25;

/// Drive the poller against `repo` until the run reaches `PrOpen`, reporting
/// each stage's result out-of-band between ticks (as a real agent would).
async fn drive_to_pr_open(
    repo: &Arc<dyn GithubIssueWorkflowRepository>,
    config: &GithubIssueWorkflowConfig,
    issue: &GithubIssueRef,
    github: Arc<RecordingGithubPort>,
) {
    let now = testing::fixed_time(2000);
    let ports = TestPollerPorts::new(Arc::clone(repo), vec![config.clone()], github);
    let poller = GithubIssueWorkflowPoller::new(ports, testing::poller_config(), POLICY_VERSION);

    for _ in 0..MAX_TICKS {
        poller.tick_once().await.expect("poller tick_once");
        let run = testing::load_run(repo.as_ref(), config, issue, POLICY_VERSION, now).await;
        if run.workflow_state.mode == GithubIssueWorkflowMode::PrOpen {
            return;
        }
        if run.active_stage_run_id.is_some() {
            testing::complete_active_stage(repo.as_ref(), &run, now).await;
        }
    }
    panic!("workflow did not reach PrOpen within {MAX_TICKS} ticks");
}

#[tokio::test]
async fn issue_discovered_runs_full_path_to_draft_pr_and_replays_idempotently_after_restart() {
    // The libSQL case is the release-blocking durable backend (embedded file
    // DB; no Docker). Postgres cases are env-gated and intentionally excluded.
    let case = RepositoryCase::cases("hermetic-e2e")
        .await
        .into_iter()
        .find(|case| case.name.starts_with("libsql"))
        .expect("libsql repository case present under the libsql feature");

    let config = testing::workflow_config("nearai", "ironclaw");
    let snapshot = testing::issue_snapshot("nearai", "ironclaw", 42);
    let issue = snapshot.issue_ref();
    let now = testing::fixed_time(2000);

    // Shared across the restart so created-PR counts accumulate — proving the
    // replay does not open a second draft PR.
    let github = Arc::new(RecordingGithubPort::new());
    github.seed_issue(snapshot.clone()).await;

    // --- Phase A: full path to a draft PR over a durable libSQL repo ---
    let repo = case.open().await;
    drive_to_pr_open(&repo, &config, &issue, Arc::clone(&github)).await;

    let run = testing::load_run(repo.as_ref(), &config, &issue, POLICY_VERSION, now).await;
    assert_eq!(
        run.workflow_state.mode,
        GithubIssueWorkflowMode::PrOpen,
        "run reached PrOpen"
    );
    assert_eq!(
        run.workflow_state.primary_pr.as_ref().map(|pr| pr.number),
        Some(testing::TEST_DRAFT_PR_NUMBER),
        "the created draft PR is recorded as the run's primary PR"
    );

    let created = github.created_prs().await;
    assert_eq!(created.len(), 1, "exactly one draft PR created");
    assert_eq!(
        created[0].head_branch,
        testing::TEST_WORKING_BRANCH,
        "draft PR head is the published working branch"
    );
    assert_eq!(
        created[0].base_branch, issue.default_branch,
        "draft PR base is the issue default branch"
    );

    // --- Phase B: restart (fresh handle, same libSQL file) + replay ---
    let reopened = case.reopen().await;
    let reloaded = testing::load_run(reopened.as_ref(), &config, &issue, POLICY_VERSION, now).await;
    assert_eq!(
        reloaded.workflow_state.mode,
        GithubIssueWorkflowMode::PrOpen,
        "durable run state survives the restart"
    );

    // Converging again over the reopened repo must be idempotent.
    drive_to_pr_open(&reopened, &config, &issue, Arc::clone(&github)).await;

    assert_eq!(
        github.created_prs().await.len(),
        1,
        "restart + replay must not create a duplicate draft PR"
    );
}
