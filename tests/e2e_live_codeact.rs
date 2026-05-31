//! Live tests that exercise specific CodeAct preamble guidelines.
//!
//! Each test drives the engine-v2 CodeAct path end-to-end with a real LLM
//! and asserts the agent followed one concrete guideline from
//! `crates/ironclaw_engine/prompts/codeact_preamble.md`:
//!
//! 1. `codeact_python_in_python` — FINAL() round-trips Python source code
//!    verbatim when the user asks for code. Exercises string-in-string
//!    escaping inside a Python script.
//! 2. `codeact_cron_reminder` — a reminder request with a concrete time
//!    creates a mission with a cron cadence pointed at that time.
//! 3. `codeact_event_mission_telegram` — "whenever you receive a Telegram
//!    message" creates a mission with an `event:telegram:...` cadence.
//! 4. `codeact_solve_github_issue` — a real public GitHub issue URL gets
//!    fetched via `http` / `web_fetch` and the agent produces a
//!    non-empty plan grounded in the fetched content.
//!
//! These are live-only (`#[ignore]`) — no committed replay fixtures yet.
//!
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql \
//!     --test e2e_live_codeact -- --ignored --test-threads=1
//! ```

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod codeact_guideline_tests {
    use std::time::Duration;

    use chrono::Datelike;

    use crate::support::live_harness::{LiveTestHarnessBuilder, TestMode};

    // ── Helpers ──────────────────────────────────────────────────────

    /// Tool-name matcher that accepts both the bare name and the
    /// argument-prefixed `"<name>(args)"` form emitted by
    /// `format_action_display_name`.
    fn used_tool(tools: &[String], expected: &str) -> bool {
        tools.iter().any(|t| {
            t == expected
                || t.strip_prefix(expected)
                    .is_some_and(|rest| rest.starts_with('('))
        })
    }

    /// Return the cadence string passed to `mission_create` — read from
    /// the tool-call detail captured by the engine, not from LLM source.
    ///
    /// The engine's `summarize_params` surfaces the cadence in the
    /// `ToolStarted` detail, which shows up in `tool_calls_started()` as
    /// `"mission_create(<cadence>)"`. This is authoritative (it's the
    /// value the tool actually ran with) and survives whether the LLM
    /// wrote the cadence as a string literal or built it in a variable.
    fn extracted_cadence(tools: &[String]) -> Option<String> {
        for t in tools {
            if let Some(rest) = t.strip_prefix("mission_create")
                && let Some(rest) = rest.strip_prefix('(')
                && let Some(arg) = rest.strip_suffix(')')
            {
                return Some(arg.to_string());
            }
        }
        None
    }

    // ── 1. Python-in-Python ─────────────────────────────────────────

    /// The user asks the agent to produce Python code. The CodeAct turn
    /// must round-trip Python *inside* a Python `FINAL(...)` call
    /// without breaking string escaping. The assertion checks that the
    /// user-facing response contains a fenced code block with plausible
    /// Python: a `sum(range(...))` idiom or a `for` loop accumulator
    /// for 1..N, plus a reference to summing to 10.
    #[tokio::test]
    #[ignore] // Live tier — no committed replay fixture.
    async fn codeact_python_in_python() {
        let harness = LiveTestHarnessBuilder::new("codeact_python_in_python")
            .with_engine_v2(true)
            .with_max_tool_iterations(10)
            .build()
            .await;

        let user_input = "I am doing a school assignment and need to add all \
                          numbers from 1 to 10. Can you give me some Python \
                          code to do it? Also, what happens if I want to sum \
                          from 1 to N? Show me the code for that too.";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(180)).await;
        assert!(!responses.is_empty(), "expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let joined = text.join("\n");
        eprintln!(
            "[PythonInPython] Response preview:\n{}",
            joined.chars().take(600).collect::<String>()
        );

        // The response must contain plausible Python that sums 1..N.
        // We don't require a fenced ```python block — the model can
        // choose bold labels + plain code or any other markdown style.
        // What matters is that the Python *literal* round-tripped out
        // of the FINAL() call intact: a `sum(range(...))` idiom, a
        // `for ... range(...)` accumulator, or the closed-form
        // `n * (n + 1) // 2` all count as Python-in-Python survival.
        let has_sum_range = joined.contains("sum(range(");
        let has_for_loop =
            joined.contains("for ") && (joined.contains("range(") || joined.contains("in range"));
        let has_closed_form = joined.contains("(n + 1)") || joined.contains("(n+1)");
        assert!(
            has_sum_range || has_for_loop || has_closed_form,
            "response should include Python for summing 1..N — \
             sum(range(...)), a for-loop, or the n*(n+1)/2 formula; \
             got: {joined}"
        );

        // The response must acknowledge the specific 1..10 case. The
        // answer (55) or the literal "10" / "1 to 10" anchors it.
        let lower = joined.to_lowercase();
        assert!(
            lower.contains("55") || lower.contains("1 to 10") || lower.contains("1..10"),
            "response must address the concrete 1..10 case; got: {joined}"
        );

        harness.finish(user_input, &text).await;
    }

    // ── 2. Cron mission ─────────────────────────────────────────────

    /// "Remind me to buy bread tomorrow at 9am" must create a mission
    /// whose cadence is a cron expression firing at 09:00 on tomorrow's
    /// calendar date. We don't pin a specific cron layout (the LLM may
    /// emit 5-, 6-, or 7-field) — we just check that the cadence string
    /// contains `9` and tomorrow's day-of-month.
    #[tokio::test]
    #[ignore] // Live tier — no committed replay fixture.
    async fn codeact_cron_reminder() {
        let harness = LiveTestHarnessBuilder::new("codeact_cron_reminder")
            .with_engine_v2(true)
            .with_max_tool_iterations(15)
            .with_auto_approve_tools(true)
            .build()
            .await;

        let user_input = "Can you remind me to buy bread tomorrow at 9am?";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(180)).await;
        assert!(!responses.is_empty(), "expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        eprintln!("[CronReminder] Tools: {tools:?}");
        eprintln!(
            "[CronReminder] Response preview: {}",
            text.join("\n").chars().take(400).collect::<String>()
        );

        assert!(
            used_tool(&tools, "mission_create") || used_tool(&tools, "routine_create"),
            "expected mission_create or routine_create to be called; got tools: {tools:?}"
        );

        if harness.mode() == TestMode::Live {
            if used_tool(&tools, "mission_create") {
                let cadence = extracted_cadence(&tools).unwrap_or_else(|| {
                    panic!(
                        "could not extract cadence from mission_create tool call. \
                         Tools started: {tools:?}"
                    )
                });
                eprintln!("[CronReminder] Extracted cadence: {cadence:?}");

                // Reject reactive / manual cadences — the user asked for a
                // one-shot scheduled reminder.
                assert!(
                    !cadence.starts_with("event:")
                        && !cadence.starts_with("webhook:")
                        && cadence.as_str() != "manual",
                    "cadence must be a cron expression, not event/webhook/manual; got: {cadence:?}"
                );

                // 9am somewhere in the expression. Matches "0 9" (5/6 field
                // `min hr` or `0 min hr`), or the bare hour field.
                let nine_am = cadence.contains(" 9 ") || cadence.starts_with("0 9 ");
                assert!(nine_am, "cadence must schedule for 9am; got: {cadence:?}");

                // Tomorrow's day-of-month must appear somewhere in the
                // cadence. We can't pin timezone cleanly, so we accept
                // either local-tomorrow or UTC-tomorrow.
                let local_tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap();
                let utc_tomorrow = chrono::Utc::now().date_naive().succ_opt().unwrap();
                let accepted_days = [local_tomorrow.day(), utc_tomorrow.day()];
                let day_ok = accepted_days.iter().any(|d| {
                    cadence.contains(&format!(" {d} ")) || cadence.ends_with(&format!(" {d}"))
                });
                assert!(
                    day_ok,
                    "cadence must pin tomorrow's day-of-month (expected one of \
                     {accepted_days:?}); got: {cadence:?}"
                );
            } else if used_tool(&tools, "routine_create") {
                let local_tomorrow = chrono::Local::now().date_naive().succ_opt().unwrap();
                let utc_tomorrow = chrono::Utc::now().date_naive().succ_opt().unwrap();
                let expected_local = format!(
                    "routine_create(0 9 {} {} *)",
                    local_tomorrow.day(),
                    local_tomorrow.month()
                );
                let expected_utc = format!(
                    "routine_create(0 9 {} {} *)",
                    utc_tomorrow.day(),
                    utc_tomorrow.month()
                );
                eprintln!(
                    "[CronReminder] Checking routine_create cadence, expected one of: \
                     {expected_local:?} or {expected_utc:?}"
                );
                assert!(
                    tools.contains(&expected_local) || tools.contains(&expected_utc),
                    "expected routine_create with cron for tomorrow at 9am \
                     (one of '{expected_local}' or '{expected_utc}'); \
                     got tools: {tools:?}"
                );
            }
        }

        harness.finish(user_input, &text).await;
    }

    // ── 3. Event mission (telegram) ────────────────────────────────

    /// "Every time you receive a telegram message please store it" must
    /// produce a mission with an `event:telegram:<regex>` cadence.
    #[tokio::test]
    #[ignore] // Live tier — no committed replay fixture.
    async fn codeact_event_mission_telegram() {
        let harness = LiveTestHarnessBuilder::new("codeact_event_mission_telegram")
            .with_engine_v2(true)
            .with_max_tool_iterations(15)
            .with_auto_approve_tools(true)
            .build()
            .await;

        let user_input = "Every time you receive a telegram message please store it.";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(180)).await;
        assert!(!responses.is_empty(), "expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        eprintln!("[EventMission] Tools: {tools:?}");
        eprintln!(
            "[EventMission] Response preview: {}",
            text.join("\n").chars().take(400).collect::<String>()
        );

        assert!(
            used_tool(&tools, "mission_create") || used_tool(&tools, "routine_create"),
            "expected mission_create or routine_create to be called; got tools: {tools:?}"
        );

        if harness.mode() == TestMode::Live {
            if used_tool(&tools, "mission_create") {
                let cadence = extracted_cadence(&tools).unwrap_or_else(|| {
                    panic!(
                        "could not extract cadence from mission_create tool call. \
                         Tools started: {tools:?}"
                    )
                });
                eprintln!("[EventMission] Extracted cadence: {cadence:?}");

                assert_eq!(
                    cadence, "event:telegram:.*",
                    "cadence must be the exact 'every telegram message' pattern \
                     from the preamble: event:telegram:.*; got: {cadence:?}"
                );
            } else if used_tool(&tools, "routine_create") {
                let telegram_routine = tools.iter().any(|t| {
                    t.starts_with("routine_create(") && t.to_ascii_lowercase().contains("telegram")
                });
                assert!(
                    telegram_routine,
                    "expected routine_create with a Telegram-related description; \
                     got tools: {tools:?}"
                );
            }
        }

        harness.finish(user_input, &text).await;
    }

    // ── 4. YAML → TOML conversion ───────────────────────────────────

    /// The user pastes a YAML config and asks for TOML. The agent must
    /// produce TOML directly as text (no code-gen step needed), preserving
    /// all nested structures, scalar values, and the features array.
    #[tokio::test]
    #[ignore] // Live tier — no committed replay fixture.
    async fn codeact_yaml_to_toml() {
        const YAML_INPUT: &str = r#"
database:
  host: localhost
  port: 5432
  name: myapp_db
  pool:
    min: 2
    max: 10

server:
  host: "0.0.0.0"
  port: 8080
  debug: false

features:
  - auth
  - logging
  - metrics
"#;

        let harness = LiveTestHarnessBuilder::new("codeact_yaml_to_toml")
            .with_engine_v2(true)
            .with_max_tool_iterations(10)
            .build()
            .await;

        let user_input = format!(
            "Convert the following YAML configuration to TOML format. \
             Preserve all nested structures and key-value pairs. \
             Return only the TOML output, no explanation needed.\n\n\
             ```yaml\n{YAML_INPUT}```"
        );
        let rig = harness.rig();
        rig.send_message(&user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(120)).await;
        assert!(!responses.is_empty(), "expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let joined = text.join("\n");
        let lower = joined.to_lowercase();
        eprintln!(
            "[YamlToToml] Response preview:\n{}",
            joined.chars().take(600).collect::<String>()
        );

        // TOML section headers or dotted keys for nested structs.
        assert!(
            lower.contains("[database]") || lower.contains("database.host"),
            "response must contain TOML database section; got: {joined}"
        );
        assert!(
            lower.contains("[server]") || lower.contains("server.host"),
            "response must contain TOML server section; got: {joined}"
        );
        // Scalar values must survive the conversion.
        assert!(
            joined.contains("5432"),
            "response must preserve database port 5432; got: {joined}"
        );
        // Array items must all appear.
        assert!(
            lower.contains("auth") && lower.contains("logging") && lower.contains("metrics"),
            "response must preserve all features (auth, logging, metrics); got: {joined}"
        );

        harness.finish(&user_input, &text).await;
    }

    // ── 5. File summarization ────────────────────────────────────────

    /// The agent reads two fixture files (a feature description and a bug
    /// report for the same rate-limiter module), writes a combined summary
    /// to /tmp, and the test reads the output file to verify it synthesises
    /// content from both inputs.
    #[tokio::test]
    #[ignore] // Live tier — no committed replay fixture.
    async fn codeact_file_summarize() {
        let fixture_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/file_summarize");
        let feature_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/file_summarize/feature.txt"
        );
        let bug_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/file_summarize/bug.txt"
        );
        let output_path = "/tmp/ironclaw_file_summarize_output.txt";

        // Remove any stale output from a previous run.
        let _ = std::fs::remove_file(output_path);

        let harness = LiveTestHarnessBuilder::new("codeact_file_summarize")
            .with_engine_v2(true)
            .with_max_tool_iterations(10)
            .with_auto_approve_tools(true)
            .build()
            .await;

        let user_input = format!(
            "Both files below are in the same directory ({fixture_dir}):\n\
             1. {feature_path}\n\
             2. {bug_path}\n\n\
             Read both files, write a concise summary that covers the feature \
             description and the bug report, then save the summary to \
             {output_path}."
        );
        let rig = harness.rig();
        rig.send_message(&user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(180)).await;
        assert!(!responses.is_empty(), "expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        eprintln!("[FileSummarize] Tools: {tools:?}");
        eprintln!(
            "[FileSummarize] Response preview: {}",
            text.join("\n").chars().take(400).collect::<String>()
        );

        assert!(
            used_tool(&tools, "read_file"),
            "expected read_file to be called; got tools: {tools:?}"
        );
        assert!(
            used_tool(&tools, "write_file"),
            "expected write_file to be called; got tools: {tools:?}"
        );

        let output = std::fs::read_to_string(output_path)
            .unwrap_or_else(|e| panic!("output file not written at {output_path}: {e}"));
        let output_lower = output.to_lowercase();

        assert!(
            output_lower.contains("rate limit")
                || output_lower.contains("rate-limit")
                || output_lower.contains("ratelimit"),
            "summary must reference the rate limiter feature; got: {output}"
        );
        assert!(
            output_lower.contains("bug")
                || output_lower.contains("issue")
                || output_lower.contains("fix"),
            "summary must reference the bug report; got: {output}"
        );

        harness.finish(&user_input, &text).await;
    }

    // ── 6. GitHub API star count ─────────────────────────────────────

    /// The agent fetches the public GitHub API for nearai/ironclaw and
    /// reports the star count. The test asserts the count extracted from
    /// the response is a plausible number (> 10 000, < 1 000 000).
    #[tokio::test]
    #[ignore] // Live tier — no committed replay fixture.
    async fn codeact_github_stars() {
        let harness = LiveTestHarnessBuilder::new("codeact_github_stars")
            .with_engine_v2(true)
            .with_max_tool_iterations(10)
            .with_auto_approve_tools(true)
            .build()
            .await;

        let user_input = "Fetch \"https://api.github.com/repos/nearai/ironclaw\" and tell me \
                          how many stars the repository has.";
        let rig = harness.rig();
        rig.send_message(user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(120)).await;
        assert!(!responses.is_empty(), "expected at least one response");

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        let joined = text.join("\n");
        eprintln!("[GithubStars] Tools: {tools:?}");
        eprintln!(
            "[GithubStars] Response preview:\n{}",
            joined.chars().take(400).collect::<String>()
        );

        // The agent must have hit the GitHub API endpoint.
        let fetched = rig.captured_status_events().into_iter().any(|evt| {
            use ironclaw::channels::StatusUpdate;
            match evt {
                StatusUpdate::ToolStarted { name, detail, .. } => {
                    let lname = name.to_ascii_lowercase();
                    let is_fetch = lname.contains("http") || lname.contains("web_fetch");
                    is_fetch
                        && detail
                            .as_deref()
                            .is_some_and(|d| d.contains("nearai/ironclaw"))
                }
                _ => false,
            }
        });
        assert!(
            fetched,
            "expected agent to fetch the GitHub API for nearai/ironclaw; \
             got tools: {tools:?}"
        );

        // Extract numbers from the response (handle comma-formatted e.g. "12,345").
        let cleaned = joined.replace(',', "");
        let star_count_plausible = cleaned
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<u64>().ok())
            .any(|n| n > 10_000 && n < 1_000_000);
        assert!(
            star_count_plausible,
            "response must contain a star count between 10 000 and 1 000 000; \
             got: {joined}"
        );

        harness.finish(user_input, &text).await;
    }

    // ── 7. Solve a real GitHub issue ────────────────────────────────

    /// Point the agent at a real public issue and assert it actually
    /// fetched the issue page before proposing a plan. We don't require
    /// the agent to open a PR or produce a perfect solution — just that
    /// it grounds its response in the fetched issue content rather than
    /// fabricating.
    #[tokio::test]
    #[ignore] // Live tier — no committed replay fixture.
    async fn codeact_solve_github_issue() {
        const ISSUE_URL: &str = "https://github.com/irondevrel/docs/issues/1";

        let harness = LiveTestHarnessBuilder::new("codeact_solve_github_issue")
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            .with_secrets(["github_token"])
            .build()
            .await;

        if harness.rig().get_secret("github_token").await.is_none() {
            eprintln!(
                "[codeact_solve_github_issue] github_token not found in \
                 ~/.ironclaw/ironclaw.db; skipping. This test opens a real PR \
                 against a public repo and cannot run without credentials."
            );
            return;
        }

        let user_input = format!(
            "You are in charge of solving {ISSUE_URL}, a public issue in the irondevrel/docs repo. Check the issue, understand the problem, modify the necessary files to fix it and open a pull request with the fix. Your job is not done until the PR is created. There are no humans in the loop to assist, so iterate until done. IMPORTANT: you do not have access to the shell `git` or `gh` commands, so you will have to work entirely remotely using the actions provided by the installed `github tool`."
        );
        let rig = harness.rig();
        rig.send_message(&user_input).await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(600)).await;

        if responses.is_empty() {
            eprintln!("\n[GithubIssue] ── NO RESPONSE — diagnostic dump ────────");
            eprintln!(
                "[GithubIssue] Tools started ({}): {:?}",
                rig.tool_calls_started().len(),
                rig.tool_calls_started()
            );
            eprintln!(
                "[GithubIssue] Tools completed: {:?}",
                rig.tool_calls_completed()
            );
            let recorded = harness.rig().captured_llm_requests();
            eprintln!("[GithubIssue] LLM requests ({}):", recorded.len());
            for (i, msgs) in recorded.iter().enumerate() {
                eprintln!("\n─── LLM turn {i} ({} messages) ───\n{msgs:?}", msgs.len());
            }
            eprintln!("\n[GithubIssue] Status event stream:");
            for evt in rig.captured_status_events() {
                eprintln!("  {evt:?}");
            }
            eprintln!("[GithubIssue] ─────────────────────────────────────────\n");
            panic!("expected at least one response");
        }

        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        let joined = text.join("\n");
        eprintln!("[GithubIssue] Tools: {tools:?}");
        eprintln!(
            "[GithubIssue] Response preview:\n{}",
            joined.chars().take(800).collect::<String>()
        );

        // The agent must have hit the issue URL with an HTTP-shaped
        // tool. `http`, `web_fetch`, or `shell` (curl/gh) are all fair
        // game. Match by tool name and the URL substring in the detail.
        let fetched_issue = rig.captured_status_events().into_iter().any(|evt| {
            use ironclaw::channels::StatusUpdate;
            match evt {
                StatusUpdate::ToolStarted { name, detail, .. } => {
                    let lname = name.to_ascii_lowercase();
                    let is_fetch = lname.contains("http")
                        || lname.contains("web_fetch")
                        || lname.contains("shell");
                    is_fetch
                        && detail
                            .as_deref()
                            .is_some_and(|d| d.contains("irondevrel/docs"))
                }
                _ => false,
            }
        });
        assert!(
            fetched_issue,
            "expected the agent to fetch {ISSUE_URL} (or a URL containing \
             'irondevrel/docs') via http/web_fetch/shell; got tools: {tools:?}"
        );

        // The response must be non-trivial and must reference the issue
        // or the repo by name. Loose check — we're not evaluating the
        // quality of the proposed fix, just that the agent didn't punt
        // with an empty FINAL.
        assert!(
            joined.len() > 120,
            "expected a substantive response; got short reply: {joined}"
        );
        let lower = joined.to_lowercase();
        assert!(
            lower.contains("issue") || lower.contains("irondevrel") || lower.contains("docs"),
            "response should reference the issue or repo; got: {joined}"
        );

        harness.finish(&user_input, &text).await;
    }
}
