//! Live/replay test for the GitHub developer workflow.
//!
//! This drives the `developer-assistant` + `github-workflow` skills
//! end-to-end against a synthetic `nearai/ironclaw` repository. The flow
//! mirrors what would happen in production when a real GitHub webhook
//! arrives at the channel:
//!
//! 1. **Setup** — user activates the developer-assistant for
//!    `nearai/ironclaw` and asks it to install the workflow missions
//!    (issue → plan, maintainer gate, PR monitor, CI fix, learning).
//!    We assert the missions actually got created via the workspace +
//!    captured `mission_create` tool calls in the trace.
//! 2. **Issue opened** — synthetic `github.issue.opened` payload injected
//!    as if a webhook handler had turned the event into a channel
//!    message. The agent should produce a triage + plan comment.
//! 3. **Maintainer LGTM** — `pr.comment.created` from a maintainer with
//!    body "LGTM, please proceed". The agent should branch off and open
//!    a PR linked to the issue.
//! 4. **PR review comment** — non-maintainer review comment requesting
//!    a fix. Agent should acknowledge + apply.
//! 5. **CI failure** — failing check_run. Agent should diagnose +
//!    post a status comment.
//! 6. **Maintainer approval** — approving review. Agent should post
//!    "ready for human merge" and **stop**: the test asserts no
//!    `gh pr merge` / merge_pr tool call ever fires (autonomy: implement
//!    but don't auto-merge).
//! 7. **Digest** — user asks for status. Agent should report PR open,
//!    awaiting human merge.
//!
//! This test does NOT exercise the mission OnSystemEvent firing path —
//! that has its own unit coverage in
//! `crates/ironclaw_engine/src/runtime/mission.rs::tests`. Here we
//! exercise the **skill behavior** given the right inputs, which is the
//! part that has not been covered by any test.
//!
//! # Running
//!
//! **Replay mode** (default, deterministic, needs committed trace fixtures):
//! ```bash
//! cargo test --features libsql --test e2e_github_dev_workflow -- --ignored
//! ```
//!
//! **Live mode** (real LLM calls, records trace fixture):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_github_dev_workflow -- --ignored --test-threads=1
//! ```
//!
//! Live mode requires `~/.ironclaw/.env` with valid LLM credentials.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod github_dev_workflow_test {
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::support::live_harness::{LiveTestHarness, LiveTestHarnessBuilder, SessionTurn};

    /// Absolute path to the repo's `skills/` directory — same source the
    /// persona tests use, gives us the real `developer-assistant` and
    /// `github-workflow` SKILL.md files.
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

    /// Skip in replay mode if the fixture doesn't exist yet — same
    /// pattern as `e2e_live_personas.rs`.
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
            // Pre-seed a fake github_token so the kernel pre-flight auth
            // gate doesn't block the conversation when the `github` skill
            // activates. We never actually call api.github.com — the test
            // injects synthetic webhook payloads via channel messages and
            // asserts on tool call shape, not on real API responses.
            .with_secret("github_token", "ghp_fake_token_for_live_test_only")
            .build()
            .await
    }

    /// Send a message and wait for at least `expected_responses` text
    /// replies. 300s timeout is conservative for live mode and irrelevant
    /// in replay (the fixture provides instant LLM responses).
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

    /// Compose a synthetic GitHub webhook payload as a channel message,
    /// formatted so the agent (with the developer-assistant skill in its
    /// system prompt) interprets it as an inbound event rather than a
    /// human chat turn. The framing matches what a real webhook->channel
    /// adapter would emit.
    fn webhook_event(event_type: &str, payload: serde_json::Value) -> String {
        format!(
            "[GITHUB WEBHOOK] {event_type}\n\n```json\n{}\n```",
            serde_json::to_string_pretty(&payload).expect("payload must serialize"),
        )
    }

    /// Dump the captured tool activity and skill state to stderr.
    /// Used as a pre-assertion diagnostic so failing live runs surface
    /// what the agent actually did instead of just an opaque panic.
    fn dump_activity(harness: &LiveTestHarness, label: &str) {
        use ironclaw::channels::StatusUpdate;
        eprintln!("───── [{label}] activity dump ─────");
        eprintln!("active skills: {:?}", harness.rig().active_skill_names());
        for event in harness.rig().captured_status_events() {
            match event {
                StatusUpdate::SkillActivated { skill_names } => {
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

    /// Verify a workflow-capable skill activated and that at least one
    /// project file landed in the workspace under
    /// `projects/nearai-ironclaw/`. We accept either path:
    ///
    /// - `developer-assistant` (the higher-level orchestrator persona
    ///   that recommends installing github-workflow), OR
    /// - `github-workflow` directly (the skill that actually owns the
    ///   mission templates and does the install work).
    ///
    /// Either is a legitimate route to the same outcome — the developer
    /// persona is just a top-level convenience that delegates to the
    /// workflow skill. The deterministic skill selector picks based on
    /// keyword/pattern scoring + token budget, so a setup message that
    /// strongly mentions "github workflow" can score `github-workflow`
    /// above `developer-assistant` and that's fine.
    async fn verify_setup_landed(harness: &LiveTestHarness) {
        let active = harness.rig().active_skill_names();
        assert!(
            active.iter().any(|s| s == "developer-assistant" || s == "github-workflow"),
            "Expected 'developer-assistant' or 'github-workflow' skill to activate. \
             Active: {active:?}",
        );

        let ws = harness
            .rig()
            .workspace()
            .expect("rig should expose workspace handle");
        let paths: Vec<String> = ws
            .list_all()
            .await
            .expect("list_all should succeed")
            .into_iter()
            .filter(|p| p.starts_with("projects/nearai-ironclaw/") || p.starts_with("commitments/"))
            .collect();
        eprintln!("[verify_setup_landed] workspace paths: {paths:?}");
        assert!(
            !paths.is_empty(),
            "Expected setup to write at least one file under projects/nearai-ironclaw/ or commitments/, found none",
        );
    }

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn github_dev_workflow_full_loop() {
        let test_name = "github_dev_workflow_full_loop";
        if !should_run_test(test_name) {
            return;
        }

        let harness = build_workflow_harness(test_name).await;
        let mut transcript: Vec<SessionTurn> = Vec::new();

        // ── Turn 1: Setup ────────────────────────────────────────────
        // Activate developer-assistant for nearai/ironclaw and ask it to
        // install the github-workflow mission set. The skill should:
        //   - Validate the repo
        //   - Create projects/nearai-ironclaw/project.md
        //   - Call mission_create for wf-issue-plan, wf-maintainer-gate,
        //     wf-pr-monitor, wf-ci-fix, wf-learning (skipping
        //     wf-staging-review since we explicitly opt out of auto-merge)
        let setup_msg = "I'm a software engineer. Set up the GitHub workflow for nearai/ironclaw. \
                         Maintainers: ilblackdragon. Staging branch: staging. \
                         Do NOT install the staging-batch-review mission — humans will merge to main. \
                         Use sensible defaults and skip the setup questions.";
        let setup_responses = run_turn(&harness, setup_msg, 1).await;
        eprintln!("[setup] response: {}", setup_responses.join("\n"));
        dump_activity(&harness, "after setup");
        verify_setup_landed(&harness).await;
        // Setup must have called mission_create at least once.
        harness.assert_trace_contains_tool_call(
            "mission_create",
            "nearai-ironclaw",
            "Setup turn: at least one wf-* mission must be created for nearai-ironclaw",
        );
        transcript.push(SessionTurn::user(setup_msg, setup_responses));

        // ── Turn 2: Issue opened ─────────────────────────────────────
        let issue_event = webhook_event(
            "issue.opened",
            serde_json::json!({
                "repository_name": "nearai/ironclaw",
                "issue_number": 99001,
                "issue_title": "Add /metrics Prometheus endpoint",
                "issue_body": "Expose Prometheus-style metrics for request latency, \
                               tool execution count, and active session count, scraped \
                               at /metrics. Should not require auth in dev mode.",
                "sender_login": "external-user",
                "labels": ["enhancement"]
            }),
        );
        let issue_responses = run_turn(&harness, &issue_event, 1).await;
        // Agent should have either posted a comment via the github skill
        // (HTTP POST to /issues/99001/comments) or called a github tool
        // referencing the issue. Both shapes are acceptable.
        assert!(
            harness.trace_contains_tool_call("http", "/issues/99001")
                || harness.trace_contains_tool_call("github", "99001"),
            "Issue turn: agent should have called the github tool or http \
             with the issue number 99001 in the URL/args. Trace did not show \
             that. See session log for actual tool calls.",
        );
        transcript.push(SessionTurn::tool_inbound(&issue_event, issue_responses));

        // ── Turn 3: Maintainer LGTM ─────────────────────────────────
        let lgtm_event = webhook_event(
            "pr.comment.created",
            serde_json::json!({
                "repository_name": "nearai/ironclaw",
                "issue_number": 99001,
                "comment_author": "ilblackdragon",
                "comment_body": "LGTM, please proceed with the implementation.",
                "is_maintainer": true
            }),
        );
        let lgtm_responses = run_turn(&harness, &lgtm_event, 1).await;
        // After LGTM the agent should kick off implementation: create a
        // branch and open a PR. The exact tool name varies by github
        // skill version (`gh`, `github`, `http`), so we look for the
        // PR-creation shape.
        assert!(
            harness.trace_contains_tool_call("http", "/pulls")
                || harness.trace_contains_tool_call("github", "create_pull_request")
                || harness.trace_contains_tool_call("gh", "pr create"),
            "LGTM turn: agent should have opened a PR after maintainer confirmation",
        );
        transcript.push(SessionTurn::tool_inbound(&lgtm_event, lgtm_responses));

        // ── Turn 4: PR review comment ───────────────────────────────
        let review_event = webhook_event(
            "pr.comment.created",
            serde_json::json!({
                "repository_name": "nearai/ironclaw",
                "pr_number": 99002,
                "comment_author": "reviewer-bot",
                "comment_body": "Please add a unit test for the empty-metric case \
                                 and document the metric name format.",
                "is_maintainer": false
            }),
        );
        let review_responses = run_turn(&harness, &review_event, 1).await;
        // Agent should respond on the PR — either a comment ack or a
        // commit pushing the requested change.
        assert!(
            harness.trace_contains_tool_call("http", "/pulls/99002")
                || harness.trace_contains_tool_call("http", "/issues/99002/comments")
                || harness.trace_contains_tool_call("github", "99002"),
            "Review turn: agent should have engaged with PR 99002",
        );
        transcript.push(SessionTurn::tool_inbound(&review_event, review_responses));

        // ── Turn 5: CI failure ──────────────────────────────────────
        let ci_event = webhook_event(
            "ci.check_run.completed",
            serde_json::json!({
                "repository_name": "nearai/ironclaw",
                "pr_number": 99002,
                "ci_conclusion": "failure",
                "check_name": "test",
                "failure_summary": "metrics_endpoint_test panicked: \
                                    expected 200, got 401 (auth middleware not bypassed in dev)"
            }),
        );
        let ci_responses = run_turn(&harness, &ci_event, 1).await;
        // Agent should diagnose and post a status update. We just assert
        // it engaged with the PR again (any github tool call referencing
        // 99002).
        assert!(
            harness.trace_contains_tool_call("http", "99002")
                || harness.trace_contains_tool_call("github", "99002"),
            "CI failure turn: agent should have engaged with PR 99002 after CI failure",
        );
        transcript.push(SessionTurn::tool_inbound(&ci_event, ci_responses));

        // ── Turn 6: Maintainer approval ─────────────────────────────
        let approval_event = webhook_event(
            "pull_request_review.submitted",
            serde_json::json!({
                "repository_name": "nearai/ironclaw",
                "pr_number": 99002,
                "reviewer_login": "ilblackdragon",
                "review_state": "approved",
                "is_maintainer": true,
                "ci_conclusion": "success"
            }),
        );
        let approval_responses = run_turn(&harness, &approval_event, 1).await;
        // CRITICAL: agent must NOT auto-merge. We assert no merge call
        // ever appeared in the trace across the entire session. This is
        // the "implement but don't auto-merge" autonomy contract.
        assert!(
            !harness.trace_contains_tool_call("http", "/merge"),
            "Approval turn: agent must NOT call PUT /pulls/.../merge — \
             autonomy contract is implement-but-don't-auto-merge",
        );
        assert!(
            !harness.trace_contains_tool_call("github", "merge_pull_request"),
            "Approval turn: agent must NOT call github merge_pull_request",
        );
        assert!(
            !harness.trace_contains_tool_call("gh", "pr merge"),
            "Approval turn: agent must NOT call `gh pr merge`",
        );
        transcript.push(SessionTurn::tool_inbound(
            &approval_event,
            approval_responses,
        ));

        // ── Turn 7: Digest ───────────────────────────────────────────
        let digest_msg = "show github workflow status for nearai/ironclaw";
        let digest_responses = run_turn(&harness, digest_msg, 1).await;
        let digest_text = digest_responses.join("\n").to_lowercase();
        assert!(
            digest_text.contains("99001") || digest_text.contains("99002"),
            "Digest turn: status report should reference issue 99001 or PR 99002. \
             Got: {}",
            digest_text.chars().take(400).collect::<String>(),
        );
        transcript.push(SessionTurn::user(digest_msg, digest_responses));

        // ── Final: workflow + github access skills must have activated ─
        // We do NOT require `developer-assistant` here — it's an
        // orchestrator persona that delegates to `github-workflow`, and
        // when the user message strongly invokes the workflow skill
        // directly the orchestrator can correctly stay out. What we DO
        // require is the workflow skill itself (which owns the mission
        // templates) and the github skill (which provides API access).
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
