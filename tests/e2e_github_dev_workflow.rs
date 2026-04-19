//! Live integration test for the GitHub developer workflow.
//!
//! This is a **fully real** end-to-end test that exercises the
//! `developer-setup` + `github-workflow` skills against the real
//! `nearai/ironclaw` GitHub repo. The intent (per project owner) is to
//! validate the workflow by doing useful work on the real repo and
//! recording every interaction so we can debug what doesn't work and
//! iterate on the skills.
//!
//! ## Flow
//!
//! 1. **Setup turn** — agent installs the workflow missions
//!    (`wf-issue-plan-*`, `wf-maintainer-gate-*`, `wf-pr-monitor-*`,
//!    `wf-ci-fix-*`, `wf-learning-*`) for `nearai/ironclaw` via real
//!    `mission_create` calls.
//!
//! 2. **Real issue creation** — the test (NOT the agent) opens a real
//!    issue on `nearai/ironclaw` via direct REST API. Title is prefixed
//!    `[live-test {timestamp}]` so it's identifiable. Issue URL is
//!    printed at the start so the human running the test can monitor
//!    or intervene.
//!
//! 3. **Triage turn** — the test tells the agent "issue #N just opened,
//!    please triage and post a plan." The agent reads the real issue,
//!    generates a plan, and posts a real comment back via the github
//!    skill.
//!
//! 4. **Verification** — the test polls the real issue's comments via
//!    REST and asserts that at least one new comment exists since the
//!    test started. Comment content is logged to the session log for
//!    human review (we don't assert on text since LLM output varies).
//!
//! 5. **Cleanup** — the test closes the real issue with a final
//!    "live-test complete" comment, regardless of pass/fail. If the
//!    test panics before reaching cleanup, the issue URL is in stderr
//!    so it can be closed manually.
//!
//! ## Why "real" instead of synthetic?
//!
//! An earlier version of this test used synthetic webhook payloads
//! injected as channel messages. That approach kept the test hermetic
//! but couldn't surface the realistic failure modes (auth gates,
//! rate limits, payload format mismatches) that show up in production.
//! Per project owner's direction, this version goes all-in on real
//! artifacts so the recorded trace becomes authoritative debug data.
//!
//! The mission `OnSystemEvent` firing path (real webhook → mission →
//! spawned thread) is NOT exercised here — that requires running an
//! HTTP server and registering a real GitHub webhook, which is out of
//! scope for this test. We exercise the **skill behavior** by driving
//! the same agent conversation that a mission thread would drive.
//!
//! ## Running
//!
//! **Live mode** (default for this test — there is no replay yet):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql \
//!     --test e2e_github_dev_workflow \
//!     -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Requires:
//! - `~/.ironclaw/.env` with valid LLM credentials
//! - A `github_token` secret in `~/.ironclaw/ironclaw.db` with `repo`
//!   scope (the test rig copies it via `with_secrets(["github_token"])`)
//!
//! Replay mode is supported but requires the trace fixture to exist.
//! On the first run with a fresh `.env` the test records the fixture.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod github_dev_workflow_test {
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::support::live_harness::{LiveTestHarness, LiveTestHarnessBuilder, SessionTurn};

    /// Repository under test. Owned and watched by the project owner;
    /// safe to create live-test issues against.
    const REPO_OWNER: &str = "nearai";
    const REPO_NAME: &str = "ironclaw";

    fn repo_skills_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills")
    }

    fn trace_fixture_path(test_name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("llm_traces")
            .join("live")
            .join(format!("{test_name}.json"))
    }

    /// Skip in replay mode if the fixture doesn't exist yet.
    fn should_run_test(test_name: &str) -> bool {
        if trace_fixture_path(test_name).exists()
            || std::env::var("IRONCLAW_LIVE_TEST")
                .ok()
                .filter(|v| !v.is_empty() && v != "0")
                .is_some()
        {
            true
        } else {
            eprintln!(
                "[{}] replay fixture missing at {}; skipping until recorded in live mode",
                test_name,
                trace_fixture_path(test_name).display()
            );
            false
        }
    }

    async fn build_workflow_harness(test_name: &str) -> LiveTestHarness {
        LiveTestHarnessBuilder::new(test_name)
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            // Workflow setup involves many sequential mission_create calls
            // plus per-event reasoning, so we need a generous iteration cap.
            .with_max_tool_iterations(80)
            .with_skills_dir(repo_skills_dir())
            // Copy the real github_token from ~/.ironclaw/ironclaw.db so
            // the agent can talk to api.github.com. Required: the test
            // creates a real issue and the agent reads it + comments on it.
            .with_secrets(["github_token"])
            .build()
            .await
    }

    /// Send a message and wait for at least `expected_responses` text replies.
    async fn run_turn(
        harness: &LiveTestHarness,
        message: &str,
        expected_responses: usize,
    ) -> Vec<String> {
        let rig = harness.rig();
        let before = rig.captured_responses().await.len();
        rig.send_message(message).await;
        let responses = rig
            .wait_for_responses(before + expected_responses, Duration::from_secs(300))
            .await;
        let new_responses: Vec<String> = responses
            .into_iter()
            .skip(before)
            .map(|r| r.content)
            .collect();
        assert!(
            !new_responses.is_empty(),
            "Expected at least one response to: {message}"
        );
        new_responses
    }

    /// Dump captured tool activity to stderr after each turn so failing
    /// runs surface what the agent actually did.
    fn dump_activity(harness: &LiveTestHarness, label: &str) {
        use ironclaw::channels::StatusUpdate;
        eprintln!("───── [{label}] activity dump ─────");
        eprintln!("active skills: {:?}", harness.rig().active_skill_names());
        for event in harness.rig().captured_status_events() {
            match event {
                StatusUpdate::SkillActivated { skill_names, .. } => {
                    eprintln!("  ◆ skills activated: {}", skill_names.join(", "));
                }
                StatusUpdate::ToolStarted { name, detail, .. } => {
                    eprintln!("  ● {name} {}", detail.unwrap_or_default());
                }
                StatusUpdate::ToolCompleted {
                    name,
                    success,
                    error,
                    ..
                } => {
                    if success {
                        eprintln!("  ✓ {name}");
                    } else {
                        eprintln!("  ✗ {name}: {}", error.unwrap_or_default());
                    }
                }
                StatusUpdate::ToolResult { name, preview, .. } => {
                    let short: String = preview.chars().take(200).collect();
                    eprintln!("    {name} → {short}");
                }
                _ => {}
            }
        }
        eprintln!("───── end activity ─────");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Direct GitHub REST helpers
    //
    // These run inside the test process (not via the agent) so the test
    // can set up real artifacts before the agent runs and verify/clean up
    // afterwards. They use the same `github_token` the agent uses (read
    // back from the rig's SecretsStore via `rig.get_secret`).
    //
    // We use reqwest directly rather than the agent's `http` tool because
    // the test needs guaranteed access to GitHub regardless of skill
    // selection / tool gating.
    // ─────────────────────────────────────────────────────────────────────

    mod github_api {
        use serde_json::Value;

        const GITHUB_API: &str = "https://api.github.com";

        fn client() -> reqwest::Client {
            reqwest::Client::builder()
                .user_agent("ironclaw-live-test/0.1")
                .build()
                .expect("build reqwest client")
        }

        /// Open a real issue. Returns `(issue_number, html_url)`.
        pub async fn create_issue(
            token: &str,
            owner: &str,
            repo: &str,
            title: &str,
            body: &str,
        ) -> Result<(u64, String), String> {
            let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues");
            let resp = client()
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "title": title,
                    "body": body,
                    "labels": ["live-test"],
                }))
                .send()
                .await
                .map_err(|e| format!("create_issue request: {e}"))?;
            let status = resp.status();
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("create_issue body: {e}"))?;
            if !status.is_success() {
                return Err(format!("create_issue {status}: {body_text}"));
            }
            let v: Value =
                serde_json::from_str(&body_text).map_err(|e| format!("create_issue parse: {e}"))?;
            let number = v
                .get("number")
                .and_then(|n| n.as_u64())
                .ok_or_else(|| format!("create_issue: no number in response: {body_text}"))?;
            let html_url = v
                .get("html_url")
                .and_then(|s| s.as_str())
                .map(String::from)
                .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/issues/{number}"));
            Ok((number, html_url))
        }

        /// List comments on an issue. Returns the raw JSON array.
        pub async fn list_issue_comments(
            token: &str,
            owner: &str,
            repo: &str,
            issue_number: u64,
        ) -> Result<Vec<Value>, String> {
            let url = format!(
                "{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}/comments?per_page=100"
            );
            let resp = client()
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| format!("list_issue_comments request: {e}"))?;
            let status = resp.status();
            let body_text = resp
                .text()
                .await
                .map_err(|e| format!("list_issue_comments body: {e}"))?;
            if !status.is_success() {
                return Err(format!("list_issue_comments {status}: {body_text}"));
            }
            let v: Value = serde_json::from_str(&body_text)
                .map_err(|e| format!("list_issue_comments parse: {e}"))?;
            Ok(v.as_array().cloned().unwrap_or_default())
        }

        /// Post a comment on an issue (used by the test for the LGTM
        /// confirmation step and for the final cleanup notice).
        pub async fn post_issue_comment(
            token: &str,
            owner: &str,
            repo: &str,
            issue_number: u64,
            body: &str,
        ) -> Result<(), String> {
            let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}/comments");
            let resp = client()
                .post(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .bearer_auth(token)
                .json(&serde_json::json!({ "body": body }))
                .send()
                .await
                .map_err(|e| format!("post_issue_comment request: {e}"))?;
            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                return Err(format!("post_issue_comment {status}: {body_text}"));
            }
            Ok(())
        }

        /// Close an issue. Used by cleanup at the end of the test.
        pub async fn close_issue(
            token: &str,
            owner: &str,
            repo: &str,
            issue_number: u64,
        ) -> Result<(), String> {
            let url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues/{issue_number}");
            let resp = client()
                .patch(&url)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .bearer_auth(token)
                .json(&serde_json::json!({ "state": "closed" }))
                .send()
                .await
                .map_err(|e| format!("close_issue request: {e}"))?;
            let status = resp.status();
            if !status.is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                return Err(format!("close_issue {status}: {body_text}"));
            }
            Ok(())
        }
    }

    /// Best-effort cleanup helper. Posts a final notice and closes the
    /// issue. Logs failures to stderr instead of panicking — cleanup
    /// should never mask the original test result.
    async fn cleanup_issue(token: &str, issue_number: u64) {
        let final_comment = "🤖 **Live test complete.** Closing this issue.\n\n\
             This issue was created by the IronClaw `e2e_github_dev_workflow` \
             live integration test. If you're seeing this and the test was \
             still useful, the recorded trace is at \
             `tests/fixtures/llm_traces/live/github_dev_workflow_full_loop.json`.";
        if let Err(e) = github_api::post_issue_comment(
            token,
            REPO_OWNER,
            REPO_NAME,
            issue_number,
            final_comment,
        )
        .await
        {
            eprintln!("[cleanup] WARNING: failed to post final comment: {e}");
        }
        if let Err(e) = github_api::close_issue(token, REPO_OWNER, REPO_NAME, issue_number).await {
            eprintln!("[cleanup] WARNING: failed to close issue #{issue_number}: {e}");
            eprintln!(
                "[cleanup] Manual cleanup needed: \
                 https://github.com/{REPO_OWNER}/{REPO_NAME}/issues/{issue_number}"
            );
        } else {
            eprintln!("[cleanup] closed issue #{issue_number}");
        }
    }

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys + github_token in ~/.ironclaw
    async fn github_dev_workflow_full_loop() {
        let test_name = "github_dev_workflow_full_loop";
        if !should_run_test(test_name) {
            return;
        }

        let harness = build_workflow_harness(test_name).await;
        let mut transcript: Vec<SessionTurn> = Vec::new();

        // Pull the github token back out of the rig's secrets store so
        // we can issue direct REST calls. The harness already attempted
        // to seed it via with_secrets(["github_token"]) during build.
        //
        // This test is **inherently live-only** for the GitHub side: it
        // creates real issues, polls real comments, and closes real
        // issues regardless of whether the LLM is replayed from a
        // fixture. If the token is missing we skip gracefully — there
        // is no useful pure-replay mode for a test whose verification
        // step is "did a real comment appear on a real GitHub issue".
        let Some(github_token) = harness.rig().get_secret("github_token").await else {
            eprintln!(
                "[{test_name}] github_token not found in ~/.ironclaw/ironclaw.db; \
                 skipping. This test makes real GitHub API calls and cannot run \
                 in pure replay mode. To enable: configure a github_token secret \
                 in your local ironclaw setup and rerun with IRONCLAW_LIVE_TEST=1."
            );
            return;
        };

        // ── Turn 1: Setup workflow ───────────────────────────────────
        // The agent installs the wf-* mission set for nearai/ironclaw.
        // We don't strictly need this turn for the triage flow below,
        // but it exercises the github-workflow skill's install path
        // which is the other half of the dev workflow surface area.
        let setup_msg = "I'm a software engineer. Set up the GitHub workflow for \
                         nearai/ironclaw. Maintainers: ilblackdragon. Staging \
                         branch: staging. Do NOT install the staging-batch-review \
                         mission — humans will merge to main. Use sensible defaults \
                         and skip the setup questions.";
        let setup_responses = run_turn(&harness, setup_msg, 1).await;
        eprintln!("[setup] response: {}", setup_responses.join("\n"));
        dump_activity(&harness, "after setup");

        // Setup must have called mission_create at least once and the
        // response must reference the wf-* templates.
        harness.assert_trace_contains_tool_call(
            "mission_create",
            "",
            "Setup turn: at least one mission_create call required",
        );
        let setup_text = setup_responses.join("\n").to_lowercase();
        assert!(
            setup_text.contains("wf-issue-plan")
                || setup_text.contains("wf-pr-monitor")
                || setup_text.contains("wf-ci-fix"),
            "Setup turn: response should mention at least one wf-* mission name. Got: {}",
            setup_text.chars().take(400).collect::<String>(),
        );
        transcript.push(SessionTurn::user(setup_msg, setup_responses));

        // ── Create a real test issue on nearai/ironclaw ──────────────
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
        let issue_title = format!("[live-test {timestamp}] Add /metrics Prometheus endpoint");
        let issue_body = "**This is an automated live integration test issue.** It was opened by \
            `tests/e2e_github_dev_workflow.rs::github_dev_workflow_full_loop` to exercise the \
            `developer-setup` + `github-workflow` skills end-to-end against a real \
            repository.\n\n\
            ## Feature request\n\n\
            Expose Prometheus-style metrics for IronClaw at `/metrics`. Should include:\n\n\
            - Request latency histograms per channel + per tool\n\
            - Tool execution count + success rate\n\
            - Active session count + thread count\n\
            - LLM token usage counters per model + per backend\n\n\
            The endpoint should not require auth in single-user mode (the typical local \
            ironclaw deployment); for multi-user gateway deployments it should require the \
            existing admin credential.\n\n\
            ## Why this matters\n\n\
            Production observability is currently limited to `tracing` log output. A \
            scrapeable metrics endpoint unlocks dashboards, alerting, and SLO tracking \
            without log-aggregation pipelines.\n\n\
            ---\n\n\
            🤖 The agent will respond to this issue with a triage and an implementation plan. \
            **The test will close this issue automatically after recording the agent's response.** \
            If you're a human reading this and the issue is still open after a few minutes, the \
            test panicked — see the test output for the last activity dump.";

        let (issue_number, issue_url) = github_api::create_issue(
            &github_token,
            REPO_OWNER,
            REPO_NAME,
            &issue_title,
            issue_body,
        )
        .await
        .expect("create real test issue on nearai/ironclaw");
        eprintln!("[live-test] created real issue #{issue_number}: {issue_url}");

        // Wrap everything after issue creation in a guard so cleanup
        // runs even if an assertion or .expect() panics. This prevents
        // orphaned issues on the real repo.
        let test_result = std::panic::AssertUnwindSafe(async {
            // Capture the comment count baseline so we can detect new
            // comments posted by the agent.
            let baseline_comments =
                github_api::list_issue_comments(&github_token, REPO_OWNER, REPO_NAME, issue_number)
                    .await
                    .expect("baseline list_issue_comments")
                    .len();
            eprintln!("[live-test] baseline comment count: {baseline_comments}");

            // ── Turn 2: Triage the real issue ────────────────────────
            let triage_msg = format!(
                "Issue #{issue_number} just opened on {REPO_OWNER}/{REPO_NAME}: \
                 \"{issue_title}\". Please read the issue, triage it, and post a \
                 comment with a concrete implementation plan. The plan should \
                 include: scope (what's in/out), milestones, risks, and a \
                 testing strategy. Use the github skill to read the issue body \
                 and to post your plan as an issue comment.",
            );
            let triage_responses = run_turn(&harness, &triage_msg, 1).await;
            eprintln!("[triage] response: {}", triage_responses.join("\n"));
            dump_activity(&harness, "after triage");
            transcript.push(SessionTurn::user(&triage_msg, triage_responses));

            // ── Verification: poll the real issue for new comments ──
            // The github API can be eventually-consistent on read-after-
            // write, so we poll for up to 30s.
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let latest_comments;
            loop {
                let snapshot = github_api::list_issue_comments(
                    &github_token,
                    REPO_OWNER,
                    REPO_NAME,
                    issue_number,
                )
                .await
                .expect("list_issue_comments after triage");
                if snapshot.len() > baseline_comments {
                    latest_comments = snapshot;
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    latest_comments = snapshot;
                    break;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            assert!(
                latest_comments.len() > baseline_comments,
                "Triage turn: expected agent to post at least one new comment on \
                 issue #{issue_number} (baseline {baseline_comments}, current {}). \
                 The agent may have replied conversationally instead of using \
                 the github skill — see the activity dump above for actual \
                 tool calls. Issue: {issue_url}",
                latest_comments.len(),
            );

            // Log new comments to stderr so the human can review what
            // the agent actually wrote — this is the most useful output
            // of the test for iterating on skill quality.
            let new_count = latest_comments.len() - baseline_comments;
            eprintln!(
                "[live-test] ✅ agent posted {new_count} new comment(s) on issue #{issue_number}"
            );
            for (i, comment) in latest_comments.iter().skip(baseline_comments).enumerate() {
                let author = comment
                    .get("user")
                    .and_then(|u| u.get("login"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("?");
                let body = comment
                    .get("body")
                    .and_then(|s| s.as_str())
                    .unwrap_or("(empty)");
                let body_preview: String = body.chars().take(800).collect();
                eprintln!(
                    "[live-test] new comment {} by @{author}:\n{body_preview}\n",
                    i + 1
                );
            }
        });

        let test_outcome = futures::FutureExt::catch_unwind(test_result).await;

        // ── Cleanup: always close the issue ──────────────────────────
        cleanup_issue(&github_token, issue_number).await;

        // Re-raise any panic so cargo test sees the failure.
        if let Err(panic_payload) = test_outcome {
            std::panic::resume_unwind(panic_payload);
        }

        // ── Final: workflow + github skills must have activated ──────
        let active = harness.rig().active_skill_names();
        for required in ["github-workflow", "github"] {
            assert!(
                active.iter().any(|s| s == required),
                "Expected skill '{required}' to activate during {test_name}. Active: {active:?}",
            );
        }

        harness.finish_turns_strict(&transcript).await;
    }
}
