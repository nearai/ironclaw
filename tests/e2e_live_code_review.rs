//! Live/replay test for `/code-review owner/repo N`.
//!
//! Drives the `code-review` skill against a real (or replayed) pull
//! request on `nearai/ironclaw` and verifies:
//!
//! 1. The `code-review` skill actually activated from the `/code-review`
//!    slash mention.
//! 2. The agent fetched the *correct* PR via the GitHub API (the URL
//!    contains `/repos/nearai/ironclaw/pulls/2483`, not some other PR).
//! 3. The response text references the PR the user asked about, so a
//!    silent-substitution regression (agent reviews the wrong PR but
//!    confidently answers) is caught.
//!
//! # Running
//!
//! **Replay mode** (default, deterministic, needs committed trace fixture):
//! ```bash
//! cargo test --features libsql --test e2e_live_code_review -- --ignored
//! ```
//!
//! **Live mode** (real LLM + real GitHub API, records/updates fixture):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql \
//!     --test e2e_live_code_review -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Live mode requires a `github_token` secret in the developer's
//! `~/.ironclaw/ironclaw.db` (read scope is enough; the PR is public).
//! Replay mode does not need any credentials — the trace fixture carries
//! the LLM side and the harness stubs HTTP interactions recorded in the
//! fixture.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod code_review_test {
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::support::live_harness::{LiveTestHarness, LiveTestHarnessBuilder};
    use ironclaw::channels::StatusUpdate;

    const TEST_NAME: &str = "code_review_pr_2483";
    const REPO_OWNER: &str = "nearai";
    const REPO_NAME: &str = "ironclaw";
    const PR_NUMBER: u64 = 2483;

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

    /// Extract the PR title from the trace fixture's HTTP exchanges.
    ///
    /// Finds the first exchange whose URL contains `pulls/{PR_NUMBER}` and
    /// whose response body parses as JSON with a `"title"` field (i.e. the
    /// PR metadata request, not the diff). This keeps the expected title in
    /// sync with the recorded fixture so drift is caught by the replay
    /// machinery rather than a stale hard-coded constant.
    fn pr_title_from_fixture(test_name: &str) -> Option<String> {
        let path = trace_fixture_path(test_name);
        let data = std::fs::read_to_string(&path).ok()?;
        let trace: serde_json::Value = serde_json::from_str(&data).ok()?;
        let expected_url_fragment = format!("pulls/{PR_NUMBER}");
        for exchange in trace.get("http_exchanges")?.as_array()? {
            let url = exchange
                .pointer("/request/url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !url.contains(&expected_url_fragment) {
                continue;
            }
            let body_str = exchange
                .pointer("/response/body")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Ok(body) = serde_json::from_str::<serde_json::Value>(body_str)
                && let Some(title) = body.get("title").and_then(|v| v.as_str())
            {
                return Some(title.to_string());
            }
        }
        None
    }

    /// Mirror of the pattern in `e2e_github_dev_workflow`: skip in replay
    /// mode unless the fixture is already committed. In live mode we
    /// always run (and the fixture gets recorded).
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

    async fn build_harness(test_name: &str) -> LiveTestHarness {
        LiveTestHarnessBuilder::new(test_name)
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            // Fetching the PR, parsing metadata, then fetching the diff
            // is at most a handful of tool calls, but the LLM may branch
            // on large diffs — give it enough headroom to finish.
            .with_max_tool_iterations(30)
            .with_skills_dir(repo_skills_dir())
            // The agent hits api.github.com. In live mode we need the
            // real token so the request is authenticated (avoids the
            // 60/hr unauthenticated rate limit). In replay mode the
            // token is unused — the trace fixture carries the response.
            .with_secrets(["github_token"])
            .build()
            .await
    }

    /// Dump activity for a failed run so CI logs show what the agent
    /// actually did.
    fn dump_activity(harness: &LiveTestHarness, label: &str) {
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

    /// End-to-end: `/code-review nearai/ironclaw 2483` must (a) activate
    /// the `code-review` skill, (b) hit
    /// `api.github.com/repos/nearai/ironclaw/pulls/2483`, and (c)
    /// produce a response naming the PR it reviewed.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn code_review_real_pr() {
        if !should_run_test(TEST_NAME) {
            return;
        }

        let harness = build_harness(TEST_NAME).await;
        let rig = harness.rig();

        let user_input = format!("/code-review {REPO_OWNER}/{REPO_NAME} {PR_NUMBER}");
        rig.send_message(&user_input).await;

        // Reviewing a real PR with diff-fetching + reasoning can take a
        // while in live mode — wait up to 5 minutes.
        let responses = rig.wait_for_responses(1, Duration::from_secs(300)).await;
        let response_text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let joined = response_text.join("\n");

        // ── Activity-level diagnostics before assertions ──────────────
        dump_activity(&harness, "code_review_real_pr");

        // Assertion 1: the code-review skill activated.
        //
        // Without this the test could pass on a generic "here's what I'd
        // do" reply that never touched the skill body. `/code-review` is
        // an explicit slash mention — selector::extract_skill_mentions
        // should force-select it regardless of score.
        let active = rig.active_skill_names();
        assert!(
            active.iter().any(|s| s == "code-review"),
            "Expected `code-review` skill to activate from the `/code-review` \
             mention. Active skills: {active:?}"
        );

        // Assertion 2: the agent actually called the `http` tool.
        //
        // The skill body tells the agent to reach api.github.com. If it
        // falls back to shell/git or hallucinates a review from training
        // data, this catches it. `tool_calls_started` decorates the name
        // with a short summary (e.g. `"http(https://.../pulls/2483)"`),
        // so we match on a prefix rather than bare equality.
        let tools = rig.tool_calls_started();
        assert!(
            tools.iter().any(|t| t == "http" || t.starts_with("http(")),
            "Expected the `http` tool to be invoked for the GitHub PR fetch. \
             Tools used: {tools:?}"
        );

        // Assertion 3: the request targeted *this* PR, not a different one.
        //
        // We inspect both ToolStarted.detail (which the http tool populates
        // with the URL summary) and ToolResult.preview (which echoes the
        // PR metadata). The ToolStarted.name also embeds the URL as
        // `http(<url>)` in this harness, so we check that too. Any of
        // the three surfaces containing `pulls/2483` proves the request
        // went to the correct endpoint. Checking only the response text
        // is not enough — the LLM could repeat the number from the prompt
        // without ever fetching the right PR.
        let expected_path = format!("pulls/{PR_NUMBER}");
        let pr_endpoint_hit = rig
            .captured_status_events()
            .iter()
            .any(|event| match event {
                StatusUpdate::ToolStarted { name, detail, .. } => {
                    let name_hit = name.contains(&expected_path);
                    let detail_hit = detail
                        .as_deref()
                        .map(|d| d.contains(&expected_path))
                        .unwrap_or(false);
                    (name.starts_with("http") || name == "http") && (name_hit || detail_hit)
                }
                StatusUpdate::ToolResult {
                    name: _, preview, ..
                } => preview.contains(&expected_path),
                _ => false,
            });
        assert!(
            pr_endpoint_hit,
            "Expected at least one http call or result referencing `{expected_path}`. \
             The agent invoked http() but did not appear to target PR #{PR_NUMBER}. \
             Full response preview: {}",
            joined.chars().take(400).collect::<String>()
        );

        // Assertion 4: the response names the PR it reviewed.
        //
        // Catches the "silent substitution" regression — agent fetches
        // the right PR but writes about a different one, or answers
        // generically without naming the PR at all.
        let lower = joined.to_lowercase();
        let names_pr =
            lower.contains(&format!("#{PR_NUMBER}")) || lower.contains(&PR_NUMBER.to_string());
        assert!(
            names_pr,
            "Response should reference PR #{PR_NUMBER}; got: {}",
            joined.chars().take(400).collect::<String>()
        );
        let names_repo = lower.contains("nearai/ironclaw") || lower.contains("ironclaw");
        assert!(
            names_repo,
            "Response should name the repo that was reviewed; got: {}",
            joined.chars().take(400).collect::<String>()
        );

        // Assertion 5: the review must reference the *real* PR content,
        // not a blank shell.
        //
        // The earlier fixture captured a green-ticket "looks good"
        // reply where every PR field was "unknown" because the LLM's
        // generated code mishandled the `http` envelope shape. Guard
        // against that class of silent-empty review by requiring:
        //   (a) the exact PR title from GitHub (so the agent actually
        //       extracted `body["title"]` instead of falling back to
        //       `"(unknown title)"`), and
        //   (b) at least one concrete `path:line` or fenced-code
        //       reference — a review with zero specifics is not a
        //       review.
        //
        // Extract the expected PR title from the trace fixture so that
        // re-recording the fixture automatically updates the expectation.
        // Falls back to a hard-coded value if the fixture is missing or
        // doesn't contain the PR metadata exchange (e.g. live mode before
        // the fixture is committed).
        let pr_title = pr_title_from_fixture(TEST_NAME).unwrap_or_else(|| {
            "feat(engine): add code execution failure categorization instrumentation".to_string()
        });
        assert!(
            joined.contains(&pr_title),
            "Response should include the real PR title \"{pr_title}\" — \
             absence usually means the agent never parsed the JSON body. \
             Got: {}",
            joined.chars().take(600).collect::<String>()
        );

        // At least one concrete file reference. The pattern is loose
        // on purpose: any of these signals a grounded review:
        //   - `path/to/file.rs` inside backticks
        //   - `path/to/file.rs:42` line reference
        //   - a fenced code block with diff content
        let has_concrete_reference = joined.contains("```")
            || joined.contains(".rs:")
            || joined.contains(".py:")
            || joined.contains(".ts:")
            || joined.contains(".md:")
            || joined.contains("crates/")
            || joined.contains("src/");
        assert!(
            has_concrete_reference,
            "Response should cite at least one concrete file or code reference. \
             A review without specifics is not a review. Got: {}",
            joined.chars().take(600).collect::<String>()
        );

        // Assertion 6: the sidebar / header must not say every field
        // is "unknown". This is the exact failure mode the earlier
        // fixture recorded, and it is the strongest indicator that
        // the `http` envelope handling is broken again.
        let unknown_markers = [
            "(unknown title)",
            "state: `unknown`",
            "base ← head: `unknown`",
            "files changed (reported): `0`",
        ];
        for marker in unknown_markers {
            assert!(
                !lower.contains(&marker.to_lowercase()),
                "Response contains the blank-shell marker {marker:?} — the \
                 agent's `http` response handling produced an empty PR \
                 snapshot. Got: {}",
                joined.chars().take(600).collect::<String>()
            );
        }

        harness.finish(&user_input, &response_text).await;
    }
}
