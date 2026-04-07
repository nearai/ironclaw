//! Live/replay tests for commitment-system persona bundles.
//!
//! Each test exercises a persona bundle (`ceo-assistant`,
//! `content-creator-assistant`, `trader-assistant`) over a multi-turn
//! conversation that goes beyond setup. The flow per persona is:
//!
//! 1. **Setup turn** — opening prompt activates the persona bundle and
//!    creates the `commitments/` workspace structure.
//! 2. **Capture turn** — a real-world input (meeting outcomes, content
//!    publication, market signal) that should be turned into commitments,
//!    signals, decisions, or pipeline items.
//! 3. **Workspace verification** — read the workspace directly via the
//!    test rig and assert that the captured items landed in the right
//!    files with the right tags.
//!
//! Every test runs through engine v2 with auto-approval enabled, loads the
//! real `./skills/` directory, and uses `finish_strict` so any tool error
//! or CodeAct SyntaxError in the trace fails the test.
//!
//! # Running
//!
//! **Replay mode** (default, deterministic, needs committed trace fixtures):
//! ```bash
//! cargo test --features libsql --test e2e_live_personas -- --ignored
//! ```
//!
//! **Live mode** (real LLM calls, records/updates trace fixtures):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live_personas -- --ignored --test-threads=1
//! ```
//!
//! Live mode requires `~/.ironclaw/.env` with valid LLM credentials and
//! runs one test at a time to avoid concurrent API pressure.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod persona_tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use crate::support::live_harness::{LiveTestHarness, LiveTestHarnessBuilder};

    /// Absolute path to the repo's `skills/` directory — the source of the
    /// committed SKILL.md files for commitments and persona bundles.
    fn repo_skills_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills")
    }

    /// Build a live harness configured for commitment/persona tests.
    ///
    /// Uses engine v2, auto-approves tool calls, loads all skills from the
    /// repo's `./skills/` dir, and bumps iteration count because the setup
    /// flow involves many sequential memory/mission tool calls.
    async fn build_persona_harness(test_name: &str) -> LiveTestHarness {
        LiveTestHarnessBuilder::new(test_name)
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            .with_max_tool_iterations(60)
            .with_skills_dir(repo_skills_dir())
            .build()
            .await
    }

    /// Send a message and wait for at least `expected_responses` text replies.
    /// 300s timeout is conservative for live mode and irrelevant in replay.
    async fn run_turn(
        harness: &LiveTestHarness,
        message: &str,
        expected_responses: usize,
    ) -> Vec<String> {
        let rig = harness.rig();
        rig.send_message(message).await;
        let responses = rig
            .wait_for_responses(expected_responses, Duration::from_secs(300))
            .await;
        assert!(
            !responses.is_empty(),
            "Expected at least one response to: {message}"
        );
        responses.into_iter().map(|r| r.content).collect()
    }

    /// Snapshot the workspace as a flat list of paths under `commitments/`.
    /// Used by post-turn assertions to verify that capture/setup actually
    /// landed files in the expected places.
    async fn workspace_paths(harness: &LiveTestHarness) -> Vec<String> {
        let ws = harness
            .rig()
            .workspace()
            .expect("rig should expose workspace handle");
        ws.list_all()
            .await
            .expect("list_all should succeed")
            .into_iter()
            .filter(|p| p.starts_with("commitments/"))
            .collect()
    }

    /// Read the contents of every commitments/ file matching `prefix`,
    /// returning a single concatenated string in lowercase. This is the
    /// substrate for "did the agent capture X" semantic checks.
    async fn read_under(harness: &LiveTestHarness, prefix: &str) -> String {
        let ws = harness
            .rig()
            .workspace()
            .expect("rig should expose workspace handle");
        let paths: Vec<String> = ws
            .list_all()
            .await
            .expect("list_all should succeed")
            .into_iter()
            .filter(|p| p.starts_with(prefix))
            .collect();
        let mut buf = String::new();
        for path in paths {
            if let Ok(doc) = ws.read(&path).await {
                buf.push_str(&format!("\n--- {path} ---\n"));
                buf.push_str(&doc.content);
            }
        }
        buf.to_lowercase()
    }

    /// Assert: at least one substring from `needles` appears in `haystack`.
    fn assert_any_present(haystack: &str, needles: &[&str], context: &str) {
        let lower = haystack.to_lowercase();
        let found: Vec<&&str> = needles
            .iter()
            .filter(|n| lower.contains(&n.to_lowercase()))
            .collect();
        assert!(
            !found.is_empty(),
            "{context}: none of {needles:?} appeared in workspace content (preview: {})",
            haystack.chars().take(400).collect::<String>(),
        );
    }

    /// Print debug summary of a turn for triage when a test fails.
    fn debug_turn(harness: &LiveTestHarness, label: &str, responses: &[String]) {
        let rig = harness.rig();
        eprintln!("[{label}] active skills: {:?}", rig.active_skill_names());
        let tools = rig.tool_calls_started();
        eprintln!("[{label}] tools ({}): {tools:?}", tools.len());
        let preview: String = responses.join("\n").chars().take(400).collect();
        eprintln!("[{label}] response preview: {preview}");
    }

    /// Verify the persona skill activated and the workspace has the
    /// commitments root structure (created by the setup flow).
    async fn verify_setup_landed(harness: &LiveTestHarness, expected_skill: &str) {
        let active = harness.rig().active_skill_names();
        assert!(
            active.iter().any(|s| s == expected_skill),
            "Expected persona skill '{expected_skill}' to activate. Active: {active:?}",
        );

        let paths = workspace_paths(harness).await;
        eprintln!("[verify_setup_landed] workspace paths: {paths:?}");
        // Setup must produce at least one commitments/ file. The agent has
        // latitude on which subdirs to create first, so accept any non-empty
        // commitments/ subtree.
        assert!(
            !paths.is_empty(),
            "Expected the persona setup to write at least one file under commitments/, found none",
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // CEO assistant: setup → meeting capture → workspace verification
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn ceo_full_workflow() {
        let harness = build_persona_harness("ceo_full_workflow").await;

        // Turn 1: setup phrasing carefully avoids "commitment system" so the
        // persona bundle outscores the generic `commitment-setup` skill.
        // The skill scoring picks at most 3 candidates and persona bundles
        // need to win on patterns/tags rather than the literal phrase.
        let setup = "I'm a CEO. Help me manage my day and track everything I'm \
                     delegating to my team — go ahead with sensible defaults, \
                     skip the configuration questions.";
        let setup_resp = run_turn(&harness, setup, 1).await;
        debug_turn(&harness, "ceo:setup", &setup_resp);
        verify_setup_landed(&harness, "ceo-assistant").await;

        // Turn 2: capture a meeting outcome with two clear delegations.
        let meeting = "I just had a 1:1 with my team. Sarah is going to deliver \
                       the Q2 budget proposal by Friday. Bob is drafting the \
                       acquisition term sheet by Tuesday next week. Capture both \
                       in the commitments system so I can follow up.";
        let meeting_resp = run_turn(&harness, meeting, 2).await;
        debug_turn(&harness, "ceo:capture", &meeting_resp);

        // Verify the commitments capture landed in the workspace. The agent
        // may file these under commitments/open/, commitments/delegations/,
        // or any sibling directory, so search the whole commitments/ subtree.
        let workspace = read_under(&harness, "commitments/").await;
        assert_any_present(
            &workspace,
            &["sarah", "q2 budget", "budget proposal"],
            "CEO meeting capture: Sarah's Q2 budget commitment",
        );
        assert_any_present(
            &workspace,
            &["bob", "term sheet", "acquisition"],
            "CEO meeting capture: Bob's term sheet commitment",
        );

        let all_responses: Vec<String> = setup_resp.into_iter().chain(meeting_resp).collect();
        harness.finish_strict(setup, &all_responses).await;
    }

    // ─────────────────────────────────────────────────────────────────────
    // Content creator: setup → publication + idea capture → verification
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn content_creator_full_workflow() {
        let harness = build_persona_harness("content_creator_full_workflow").await;

        // Turn 1: persona-flavored phrasing so content-creator-assistant
        // outscores the generic commitment-setup skill. Avoid "commitment"
        // / "set up commitment" wording.
        let setup = "I'm a YouTuber. Help me manage my content pipeline and \
                     publishing schedule across YouTube, TikTok, and Twitter — \
                     go ahead with sensible defaults, no configuration questions.";
        let setup_resp = run_turn(&harness, setup, 1).await;
        debug_turn(&harness, "creator:setup", &setup_resp);
        verify_setup_landed(&harness, "content-creator-assistant").await;

        // Turn 2: report a publication that should kick off distribution
        // commitments + a parked idea.
        let activity = "I just published episode 47 'AI Coding Tools' on YouTube. \
                        I need to do TikTok cuts and a Twitter thread by tomorrow \
                        evening. Also park this idea for later: a series on \
                        debugging legacy code.";
        let activity_resp = run_turn(&harness, activity, 2).await;
        debug_turn(&harness, "creator:capture", &activity_resp);

        let workspace = read_under(&harness, "commitments/").await;

        // The published episode should appear somewhere — pipeline file,
        // commitment, or signals.
        assert_any_present(
            &workspace,
            &["episode 47", "ai coding tools", "youtube"],
            "Creator capture: published episode",
        );

        // Distribution work should be tracked.
        assert_any_present(
            &workspace,
            &["tiktok", "twitter"],
            "Creator capture: distribution platforms",
        );

        // Parked idea should land somewhere identifiable.
        assert_any_present(
            &workspace,
            &["debugging legacy", "legacy code"],
            "Creator capture: parked idea",
        );

        let all_responses: Vec<String> = setup_resp.into_iter().chain(activity_resp).collect();
        harness.finish_strict(setup, &all_responses).await;
    }

    // ─────────────────────────────────────────────────────────────────────
    // Trader: setup → market signal + decision journal → verification
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn trader_full_workflow() {
        let harness = build_persona_harness("trader_full_workflow").await;

        // Turn 1: persona-flavored phrasing so trader-assistant outscores
        // the generic commitment-setup skill. Avoid "commitment" wording.
        let setup = "I'm a trader. Help me track my positions and journal my \
                     trading decisions — go ahead with sensible defaults for US \
                     equities and options. My current positions: AAPL 500 shares \
                     at $175, SPY April 520 puts (10 contracts). Skip the \
                     configuration questions.";
        let setup_resp = run_turn(&harness, setup, 1).await;
        debug_turn(&harness, "trader:setup", &setup_resp);
        verify_setup_landed(&harness, "trader-assistant").await;

        // Turn 2: a market signal AND a closed-position decision.
        let signal = "AAPL just announced a major chip partnership with TSMC. \
                      Could move significantly on tomorrow's earnings. Also: I \
                      closed my SPY puts at $4.20 this morning, the macro hedge \
                      thesis played out. Capture both — the signal needs triage \
                      and the close needs to go in the trade journal.";
        let signal_resp = run_turn(&harness, signal, 2).await;
        debug_turn(&harness, "trader:capture", &signal_resp);

        let workspace = read_under(&harness, "commitments/").await;

        // The AAPL signal should land in signals/ or open/ — somewhere
        // searchable for the next triage cycle.
        assert_any_present(
            &workspace,
            &["aapl", "tsmc", "chip partnership"],
            "Trader capture: AAPL signal",
        );

        // The SPY close should be journaled as a decision.
        assert_any_present(
            &workspace,
            &["spy", "puts", "macro hedge", "closed"],
            "Trader capture: SPY puts close",
        );

        let all_responses: Vec<String> = setup_resp.into_iter().chain(signal_resp).collect();
        harness.finish_strict(setup, &all_responses).await;
    }
}
