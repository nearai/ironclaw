//! Live/replay tests for commitment-system persona bundles.
//!
//! Each test exercises a persona bundle (`ceo-assistant`,
//! `content-creator-assistant`, `trader-assistant`, `developer-assistant`)
//! over a multi-turn conversation that goes beyond setup. The flow per
//! persona is:
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
    use tokio::time::{Instant, sleep};

    fn trace_fixture_path(test_name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("llm_traces")
            .join("live")
            .join(format!("{test_name}.json"))
    }

    /// Build a live harness configured for commitment/persona tests.
    ///
    /// Uses engine v2, auto-approves tool calls, and bumps iteration count
    /// because the setup flow involves many sequential memory/mission tool
    /// calls.
    async fn build_persona_harness(test_name: &str) -> LiveTestHarness {
        LiveTestHarnessBuilder::new(test_name)
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            .with_max_tool_iterations(60)
            .build()
            .await
    }

    struct PersonaCheck {
        needles: &'static [&'static str],
        context: &'static str,
    }

    struct WorkflowTurn {
        label: &'static str,
        message: &'static str,
        expected_responses: usize,
        checks: &'static [PersonaCheck],
    }

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

    /// Send a message and wait for at least `expected_responses` text replies.
    /// 300s timeout is conservative for live mode and irrelevant in replay.
    async fn run_turn(
        harness: &LiveTestHarness,
        message: &str,
        expected_responses: usize,
    ) -> Vec<String> {
        let rig = harness.rig();
        let before = rig.wait_for_responses(0, Duration::ZERO).await.len();
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

    async fn wait_for_check(harness: &LiveTestHarness, check: &PersonaCheck, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let workspace = read_under(harness, "commitments/").await;
            let lower = workspace.to_lowercase();
            let found = check
                .needles
                .iter()
                .any(|needle| lower.contains(&needle.to_lowercase()));
            if found {
                return;
            }
            if Instant::now() >= deadline {
                assert_any_present(&workspace, check.needles, check.context);
                return;
            }
            sleep(Duration::from_millis(200)).await;
        }
    }

    /// Print debug summary of a turn for triage when a test fails.
    fn debug_turn(
        harness: &LiveTestHarness,
        label: &str,
        responses: &[String],
        status_start: usize,
    ) {
        let rig = harness.rig();
        let events = rig.captured_status_events();
        let new_events = &events[status_start..];
        let active_skills: Vec<Vec<String>> = new_events
            .iter()
            .filter_map(|event| match event {
                ironclaw::channels::StatusUpdate::SkillActivated { skill_names, .. } => {
                    Some(skill_names.clone())
                }
                _ => None,
            })
            .collect();
        let tools: Vec<String> = new_events
            .iter()
            .filter_map(|event| match event {
                ironclaw::channels::StatusUpdate::ToolStarted { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        eprintln!("[{label}] active skills: {active_skills:?}");
        eprintln!("[{label}] tools ({}): {tools:?}", tools.len());
        let preview: String = responses.join("\n").chars().take(400).collect();
        eprintln!("[{label}] response preview: {preview}");
    }

    /// Collect all unique skill names from `SkillActivated` status events.
    fn collect_active_skill_names(harness: &LiveTestHarness) -> Vec<String> {
        let mut names: Vec<String> = harness
            .rig()
            .captured_status_events()
            .iter()
            .filter_map(|event| match event {
                ironclaw::channels::StatusUpdate::SkillActivated { skill_names, .. } => {
                    Some(skill_names.clone())
                }
                _ => None,
            })
            .flatten()
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Verify the persona skill activated and the workspace has the
    /// commitments root structure (created by the setup flow).
    async fn verify_setup_landed(harness: &LiveTestHarness, expected_skill: &str) {
        let active = collect_active_skill_names(harness);
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

    async fn run_multi_turn_workflow(
        test_name: &str,
        persona_skill: &str,
        debug_prefix: &str,
        turns: &[WorkflowTurn],
        required_skills: &[&str],
    ) {
        let harness = build_persona_harness(test_name).await;
        let mut transcript = Vec::new();

        for (idx, turn) in turns.iter().enumerate() {
            let status_start = harness.rig().captured_status_events().len();
            let responses = run_turn(&harness, turn.message, turn.expected_responses).await;
            debug_turn(
                &harness,
                &format!("{}:{}:{}", debug_prefix, idx + 1, turn.label),
                &responses,
                status_start,
            );
            if idx == 0 {
                verify_setup_landed(&harness, persona_skill).await;
            }
            if !turn.checks.is_empty() {
                for check in turn.checks {
                    wait_for_check(&harness, check, Duration::from_secs(10)).await;
                }
            }
            transcript.push((turn.message.to_string(), responses));
        }

        let active = collect_active_skill_names(&harness);
        for required in required_skills {
            assert!(
                active.iter().any(|skill| skill == required),
                "Expected skill '{required}' to activate during {test_name}. Active: {active:?}",
            );
        }

        harness.finish_turns(&transcript).await;
    }

    const CEO_SETUP_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["executive commitment calibration", "delegated", "decisions"],
        context: "CEO setup: executive commitments workspace created",
    }];
    const CEO_BUDGET_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["sarah", "q2 budget", "budget proposal"],
        context: "CEO workflow: Sarah budget delegation tracked",
    }];
    const CEO_TERM_SHEET_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["bob", "term sheet", "acquisition"],
        context: "CEO workflow: Bob term sheet delegation tracked",
    }];
    const CEO_REPLY_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["board", "reply", "monday morning"],
        context: "CEO workflow: board reply commitment tracked",
    }];
    const CEO_DECISION_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["toronto", "leadership summit", "new york"],
        context: "CEO workflow: summit decision captured",
    }];
    const CEO_IDEA_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["leadership offsite", "quarterly", "ops reviews"],
        context: "CEO workflow: first parked idea captured",
    }];
    const CEO_LEGAL_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["legal", "contract review", "thursday"],
        context: "CEO workflow: legal follow-up tracked",
    }];
    const CEO_RUNWAY_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["finance", "runway analysis", "monday"],
        context: "CEO workflow: finance delegation tracked",
    }];
    const CEO_REVIEW_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["review", "budget draft", "friday afternoon"],
        context: "CEO workflow: personal review commitment tracked",
    }];
    const CEO_SECOND_IDEA_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["skip-level", "engineering", "sales"],
        context: "CEO workflow: second parked idea captured",
    }];
    const CEO_STRATEGY_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["reforecast", "international expansion", "strategy"],
        context: "CEO workflow: strategy signal captured",
    }];
    const CEO_RESOLVED_BOARD_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["resolved_by: user", "board"],
        context: "CEO workflow: board reply resolved",
    }];
    const CEO_WORKFLOW_TURNS: &[WorkflowTurn] = &[
        WorkflowTurn {
            label: "setup",
            message: "I'm a CEO. Help me manage my day and track everything I'm delegating to my team. Use sensible defaults and skip the configuration questions.",
            expected_responses: 1,
            checks: CEO_SETUP_CHECKS,
        },
        WorkflowTurn {
            label: "delegate_budget",
            message: "Track this: Sarah is going to deliver the Q2 budget proposal by Friday.",
            expected_responses: 1,
            checks: CEO_BUDGET_CHECKS,
        },
        WorkflowTurn {
            label: "delegate_term_sheet",
            message: "Track this separately: Bob is drafting the acquisition term sheet by Tuesday next week.",
            expected_responses: 1,
            checks: CEO_TERM_SHEET_CHECKS,
        },
        WorkflowTurn {
            label: "digest_1",
            message: "show commitments",
            expected_responses: 1,
            checks: CEO_TERM_SHEET_CHECKS,
        },
        WorkflowTurn {
            label: "board_reply",
            message: "I need to reply to the board about hiring plans before Monday morning.",
            expected_responses: 1,
            checks: CEO_REPLY_CHECKS,
        },
        WorkflowTurn {
            label: "decision_capture",
            message: "Record this decision: we decided to do the leadership summit in Toronto instead of New York because the budget is tighter than expected.",
            expected_responses: 1,
            checks: CEO_DECISION_CHECKS,
        },
        WorkflowTurn {
            label: "park_idea_1",
            message: "park this idea: a quarterly leadership offsite focused on cross-functional ops reviews",
            expected_responses: 1,
            checks: CEO_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "owed_status_1",
            message: "who owes me what?",
            expected_responses: 1,
            checks: CEO_TERM_SHEET_CHECKS,
        },
        WorkflowTurn {
            label: "legal_followup",
            message: "Track this: I'm waiting on legal for the enterprise contract review by Thursday.",
            expected_responses: 1,
            checks: CEO_LEGAL_CHECKS,
        },
        WorkflowTurn {
            label: "digest_2",
            message: "show commitments",
            expected_responses: 1,
            checks: CEO_LEGAL_CHECKS,
        },
        WorkflowTurn {
            label: "finance_runway",
            message: "Track this delegation: ask finance to prepare a runway analysis for Monday.",
            expected_responses: 1,
            checks: CEO_RUNWAY_CHECKS,
        },
        WorkflowTurn {
            label: "digest_3",
            message: "show commitments",
            expected_responses: 1,
            checks: CEO_RUNWAY_CHECKS,
        },
        WorkflowTurn {
            label: "review_budget",
            message: "Track this as a separate personal commitment: I need to review Sarah's Q2 budget draft myself by Friday afternoon.",
            expected_responses: 1,
            checks: CEO_REVIEW_CHECKS,
        },
        WorkflowTurn {
            label: "park_idea_2",
            message: "park this idea separately in parked ideas: a quarterly skip-level lunch series with engineering and sales",
            expected_responses: 1,
            checks: CEO_SECOND_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "parked_ideas",
            message: "show parked ideas",
            expected_responses: 1,
            checks: CEO_SECOND_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "passive_signal",
            message: "Track this commitment: I need to review the strategy team's international expansion reforecast this week.",
            expected_responses: 1,
            checks: CEO_STRATEGY_CHECKS,
        },
        WorkflowTurn {
            label: "digest_4",
            message: "show commitments",
            expected_responses: 1,
            checks: CEO_STRATEGY_CHECKS,
        },
        WorkflowTurn {
            label: "budget_update",
            message: "Sarah got back to me about the Q2 budget proposal and the draft is in.",
            expected_responses: 1,
            checks: CEO_BUDGET_CHECKS,
        },
        WorkflowTurn {
            label: "term_sheet_update",
            message: "Bob delivered the acquisition term sheet this afternoon.",
            expected_responses: 1,
            checks: CEO_TERM_SHEET_CHECKS,
        },
        WorkflowTurn {
            label: "resolve_board",
            message: "done with the board hiring reply",
            expected_responses: 1,
            checks: CEO_RESOLVED_BOARD_CHECKS,
        },
        WorkflowTurn {
            label: "owed_status_2",
            message: "who owes me what?",
            expected_responses: 1,
            checks: CEO_RUNWAY_CHECKS,
        },
    ];

    const CREATOR_SETUP_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["content pipeline", "creator", "parked ideas"],
        context: "Creator setup: content pipeline workspace created",
    }];
    const CREATOR_PIPELINE_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["episode 48", "state of ai code review", "content-pipeline"],
        context: "Creator workflow: pipeline item captured",
    }];
    const CREATOR_SCRIPT_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["script", "episode 48", "state of ai code review"],
        context: "Creator workflow: script progress tracked",
    }];
    const CREATOR_EDIT_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["thumbnail", "final edit", "tomorrow evening"],
        context: "Creator workflow: edit and thumbnail commitments tracked",
    }];
    const CREATOR_IDEA_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["debugging legacy code", "series"],
        context: "Creator workflow: first parked idea captured",
    }];
    const CREATOR_DECISION_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["sponsor-free", "editorial", "branded content"],
        context: "Creator workflow: editorial decision captured",
    }];
    const CREATOR_PUBLISH_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["published", "episode 48", "youtube"],
        context: "Creator workflow: publication recorded",
    }];
    const CREATOR_DISTRIBUTION_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["tiktok", "twitter", "episode 48"],
        context: "Creator workflow: distribution commitments tracked",
    }];
    const CREATOR_RESOLVED_TWITTER_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["resolved_by: user", "twitter"],
        context: "Creator workflow: twitter commitment resolved",
    }];
    const CREATOR_PROMOTED_IDEA_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["claude vs gpt", "open", "coding workflows"],
        context: "Creator workflow: parked idea promoted to active commitment",
    }];
    const CREATOR_FINAL_DIGEST_CHECKS: &[PersonaCheck] = &[
        PersonaCheck {
            needles: &["claude vs gpt", "open", "coding workflows"],
            context: "Creator workflow: parked idea promoted to active commitment",
        },
        PersonaCheck {
            needles: &["sponsored", "figma", "friday"],
            context: "Creator workflow: sponsored commitment tracked",
        },
    ];
    const CONTENT_CREATOR_WORKFLOW_TURNS: &[WorkflowTurn] = &[
        WorkflowTurn {
            label: "setup",
            message: "I'm a YouTuber. Help me manage my content pipeline and publishing schedule across YouTube, TikTok, and Twitter. Use sensible defaults and skip the configuration questions.",
            expected_responses: 1,
            checks: CREATOR_SETUP_CHECKS,
        },
        WorkflowTurn {
            label: "new_piece",
            message: "Track this new content piece in the pipeline: Episode 48 State of AI Code Review",
            expected_responses: 1,
            checks: CREATOR_PIPELINE_CHECKS,
        },
        WorkflowTurn {
            label: "script_progress",
            message: "Update the content pipeline: I just finished the script for Episode 48 State of AI Code Review.",
            expected_responses: 1,
            checks: CREATOR_SCRIPT_CHECKS,
        },
        WorkflowTurn {
            label: "digest_1",
            message: "show commitments",
            expected_responses: 1,
            checks: CREATOR_SCRIPT_CHECKS,
        },
        WorkflowTurn {
            label: "edit_and_thumbnail",
            message: "Track these deadlines only: I need the thumbnail and final edit for Episode 48 wrapped by tomorrow evening. Do not create assets.",
            expected_responses: 1,
            checks: CREATOR_EDIT_CHECKS,
        },
        WorkflowTurn {
            label: "park_idea_1",
            message: "park this idea: a mini-series on debugging legacy code in real products",
            expected_responses: 1,
            checks: CREATOR_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "decision_capture",
            message: "Record this decision: keep Episode 48 sponsor-free and editorial instead of forcing branded content because the fit is weak.",
            expected_responses: 1,
            checks: CREATOR_DECISION_CHECKS,
        },
        WorkflowTurn {
            label: "parked_ideas_1",
            message: "show parked ideas",
            expected_responses: 1,
            checks: CREATOR_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "publish",
            message: "I published Episode 48 State of AI Code Review on YouTube this afternoon.",
            expected_responses: 1,
            checks: CREATOR_PUBLISH_CHECKS,
        },
        WorkflowTurn {
            label: "digest_2",
            message: "show commitments",
            expected_responses: 1,
            checks: CREATOR_PUBLISH_CHECKS,
        },
        WorkflowTurn {
            label: "distribution",
            message: "Track this commitment only: I need TikTok cuts and a Twitter thread for Episode 48 by tomorrow morning.",
            expected_responses: 1,
            checks: CREATOR_DISTRIBUTION_CHECKS,
        },
        WorkflowTurn {
            label: "park_idea_2",
            message: "park this idea separately in parked ideas: a comparison video on Claude vs GPT coding workflows",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "digest_3",
            message: "show commitments",
            expected_responses: 1,
            checks: CREATOR_DISTRIBUTION_CHECKS,
        },
        WorkflowTurn {
            label: "trend_signal",
            message: "Track this commitment: create a short take on the React compiler trend tonight.",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "sponsored_deadline",
            message: "Track this: the sponsored Figma workflow video has to ship by Friday.",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "parked_ideas_2",
            message: "what's on the backburner?",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "resolve_twitter",
            message: "done with the Twitter thread for Episode 48",
            expected_responses: 1,
            checks: CREATOR_RESOLVED_TWITTER_CHECKS,
        },
        WorkflowTurn {
            label: "resolve_tiktok",
            message: "done with the TikTok cuts for Episode 48",
            expected_responses: 1,
            checks: CREATOR_DISTRIBUTION_CHECKS,
        },
        WorkflowTurn {
            label: "promote_idea",
            message: "Let's do the Claude vs GPT coding workflows idea next.",
            expected_responses: 1,
            checks: CREATOR_PROMOTED_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "digest_4",
            message: "show commitments",
            expected_responses: 1,
            checks: CREATOR_FINAL_DIGEST_CHECKS,
        },
        WorkflowTurn {
            label: "parked_ideas_3",
            message: "show parked ideas",
            expected_responses: 1,
            checks: CREATOR_IDEA_CHECKS,
        },
    ];

    const TRADER_SETUP_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["positions", "trade journal", "trader calibration"],
        context: "Trader setup: trading workspace created",
    }];
    const TRADER_RESEARCH_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["nvda", "earnings thesis", "thursday"],
        context: "Trader workflow: research commitment tracked",
    }];
    const TRADER_AAPL_SIGNAL_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["aapl", "tsmc", "chip partnership"],
        context: "Trader workflow: bullish AAPL signal captured",
    }];
    const TRADER_DECISION_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["sold half", "aapl", "repriced"],
        context: "Trader workflow: trade decision captured",
    }];
    const TRADER_IDEA_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["pre-market checklist", "semis"],
        context: "Trader workflow: parked idea captured",
    }];
    const TRADER_DELEGATION_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["jenna", "macro note", "fed meeting"],
        context: "Trader workflow: delegated macro note tracked",
    }];
    const TRADER_CONFLICT_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["tsmc", "supplier checks", "weak"],
        context: "Trader workflow: contradictory signal captured",
    }];
    const TRADER_SPY_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["spy puts", "roll", "friday"],
        context: "Trader workflow: SPY puts decision commitment tracked",
    }];
    const TRADER_SECOND_IDEA_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["overnight hedge", "cpi weeks"],
        context: "Trader workflow: second parked idea captured",
    }];
    const TRADER_RESOLVED_RESEARCH_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["resolved_by: user", "nvda"],
        context: "Trader workflow: research commitment resolved",
    }];
    const TRADER_SECOND_DECISION_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["closed the rest", "aapl", "played out"],
        context: "Trader workflow: second trade decision captured",
    }];
    const TRADER_WORKFLOW_TURNS: &[WorkflowTurn] = &[
        WorkflowTurn {
            label: "setup",
            message: "I'm a trader. Help me track my positions and journal my trading decisions. Use sensible defaults for US equities and options. My current positions: AAPL 500 shares at $175 and SPY April 520 puts for hedging. Skip the configuration questions.",
            expected_responses: 1,
            checks: TRADER_SETUP_CHECKS,
        },
        WorkflowTurn {
            label: "digest_1",
            message: "show commitments",
            expected_responses: 1,
            checks: TRADER_SETUP_CHECKS,
        },
        WorkflowTurn {
            label: "research_commitment",
            message: "Track this: review the NVDA earnings thesis before Thursday's open.",
            expected_responses: 1,
            checks: TRADER_RESEARCH_CHECKS,
        },
        WorkflowTurn {
            label: "aapl_signal",
            message: "AAPL just announced a major chip partnership with TSMC and it could move hard on tomorrow's earnings.",
            expected_responses: 1,
            checks: TRADER_AAPL_SIGNAL_CHECKS,
        },
        WorkflowTurn {
            label: "digest_2",
            message: "show commitments",
            expected_responses: 1,
            checks: TRADER_AAPL_SIGNAL_CHECKS,
        },
        WorkflowTurn {
            label: "decision_capture",
            message: "Record this decision: I sold half my AAPL because the partnership already repriced most of the upside.",
            expected_responses: 1,
            checks: TRADER_DECISION_CHECKS,
        },
        WorkflowTurn {
            label: "park_idea_1",
            message: "park this idea: build a pre-market checklist for semis so I stop missing supplier read-throughs",
            expected_responses: 1,
            checks: TRADER_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "parked_ideas_1",
            message: "show parked ideas",
            expected_responses: 1,
            checks: TRADER_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "delegation",
            message: "I'm waiting on Jenna to send the macro note before the Fed meeting.",
            expected_responses: 1,
            checks: TRADER_DELEGATION_CHECKS,
        },
        WorkflowTurn {
            label: "digest_3",
            message: "show commitments",
            expected_responses: 1,
            checks: TRADER_DELEGATION_CHECKS,
        },
        WorkflowTurn {
            label: "conflicting_signal",
            message: "TSMC supplier checks just came in weak, which could undermine the AAPL thesis.",
            expected_responses: 1,
            checks: TRADER_CONFLICT_CHECKS,
        },
        WorkflowTurn {
            label: "digest_4",
            message: "show commitments",
            expected_responses: 1,
            checks: TRADER_CONFLICT_CHECKS,
        },
        WorkflowTurn {
            label: "spy_decision",
            message: "Track this as an open commitment in the workspace: decide whether to roll the SPY puts before Friday expiry.",
            expected_responses: 1,
            checks: TRADER_SPY_CHECKS,
        },
        WorkflowTurn {
            label: "digest_5",
            message: "show commitments",
            expected_responses: 1,
            checks: TRADER_SPY_CHECKS,
        },
        WorkflowTurn {
            label: "delegation_update",
            message: "Jenna got back to me about the macro note ahead of the Fed meeting.",
            expected_responses: 1,
            checks: TRADER_DELEGATION_CHECKS,
        },
        WorkflowTurn {
            label: "resolve_research",
            message: "done with the NVDA earnings thesis review",
            expected_responses: 1,
            checks: TRADER_RESOLVED_RESEARCH_CHECKS,
        },
        WorkflowTurn {
            label: "park_idea_2",
            message: "save for later: compare overnight hedge rules for CPI weeks",
            expected_responses: 1,
            checks: TRADER_SECOND_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "parked_ideas_2",
            message: "what's on the backburner?",
            expected_responses: 1,
            checks: TRADER_SECOND_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "second_decision",
            message: "Record this decision: I closed the rest of my AAPL today and the thesis was fully played out.",
            expected_responses: 1,
            checks: TRADER_SECOND_DECISION_CHECKS,
        },
        WorkflowTurn {
            label: "digest_6",
            message: "show commitments",
            expected_responses: 1,
            checks: TRADER_SECOND_DECISION_CHECKS,
        },
        WorkflowTurn {
            label: "parked_ideas_3",
            message: "show parked ideas",
            expected_responses: 1,
            checks: TRADER_IDEA_CHECKS,
        },
    ];

    const DEV_SETUP_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["commitments/readme", "developer calibration", "tech debt"],
        context: "Developer setup: commitments workspace and calibration",
    }];
    const DEV_CI_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["payments-api", "billing integration", "ci"],
        context: "Developer workflow: CI incident captured",
    }];
    const DEV_TECH_DEBT_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["webhook retry", "429", "tech-debt"],
        context: "Developer workflow: webhook retry debt tracked",
    }];
    const DEV_DIGEST_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["payments-api", "webhook retry"],
        context: "Developer workflow: digest reflects active work",
    }];
    const DEV_IDEA_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &[
            "flaky-test quarantine dashboard",
            "flaky-test",
            "quarantine",
        ],
        context: "Developer workflow: parked idea captured",
    }];
    const DEV_DECISION_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["billing retries", "queue", "inline request retries"],
        context: "Developer workflow: architecture decision captured",
    }];
    const DEV_DELEGATION_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["maya", "auth pr", "waiting"],
        context: "Developer workflow: delegation captured",
    }];
    const DEV_PROMOTED_DEBT_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["tags: [tech-debt]", "webhook retry"],
        context: "Developer workflow: tech debt promoted to commitment",
    }];
    const DEV_PLAN_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["webhook retry"],
        context: "Developer workflow: plan references promoted debt",
    }];
    const DEV_RESOLVED_CI_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["resolved_by: user", "payments-api"],
        context: "Developer workflow: CI commitment resolved",
    }];
    const DEV_RESOLVED_DEBT_CHECKS: &[PersonaCheck] = &[PersonaCheck {
        needles: &["type: tech-debt", "webhook retry"],
        context: "Developer workflow: debt resolution archived",
    }];
    const DEVELOPER_WORKFLOW_TURNS: &[WorkflowTurn] = &[
        WorkflowTurn {
            label: "setup",
            message: "I'm a software engineer. Help me manage my engineering commitments and PR work. Use sensible defaults and skip the setup questions.",
            expected_responses: 1,
            checks: DEV_SETUP_CHECKS,
        },
        WorkflowTurn {
            label: "explicit_commitment",
            message: "Track this: CI is red on my payments-api branch because the billing integration tests are failing, and I need it fixed today.",
            expected_responses: 1,
            checks: DEV_CI_CHECKS,
        },
        WorkflowTurn {
            label: "tech_debt_capture",
            message: "This is a hack: the webhook retry helper swallows 429s and we should refactor it later.",
            expected_responses: 1,
            checks: DEV_TECH_DEBT_CHECKS,
        },
        WorkflowTurn {
            label: "show_commitments_1",
            message: "show commitments",
            expected_responses: 1,
            checks: DEV_DIGEST_CHECKS,
        },
        WorkflowTurn {
            label: "park_idea_1",
            message: "park this idea: build a flaky-test quarantine dashboard for CI triage",
            expected_responses: 1,
            checks: DEV_IDEA_CHECKS,
        },
        WorkflowTurn {
            label: "decision_capture",
            message: "Record this decision in the workspace: move billing retries behind a queue instead of inline request retries because it isolates webhook latency spikes.",
            expected_responses: 1,
            checks: DEV_DECISION_CHECKS,
        },
        WorkflowTurn {
            label: "show_commitments_2",
            message: "show commitments",
            expected_responses: 1,
            checks: DEV_DIGEST_CHECKS,
        },
        WorkflowTurn {
            label: "delegation_capture",
            message: "I'm waiting on Maya to review the admin auth PR by Thursday, so track that follow-up too.",
            expected_responses: 1,
            checks: DEV_DELEGATION_CHECKS,
        },
        WorkflowTurn {
            label: "show_commitments_3",
            message: "show commitments",
            expected_responses: 1,
            checks: DEV_DELEGATION_CHECKS,
        },
        WorkflowTurn {
            label: "show_tech_debt_1",
            message: "show tech debt",
            expected_responses: 1,
            checks: DEV_TECH_DEBT_CHECKS,
        },
        WorkflowTurn {
            label: "promote_debt",
            message: "Let's fix the webhook retry helper debt.",
            expected_responses: 1,
            checks: DEV_PROMOTED_DEBT_CHECKS,
        },
        WorkflowTurn {
            label: "plan_create",
            message: "[PLAN MODE] Create a plan for refactoring the webhook retry helper safely with tests.",
            expected_responses: 1,
            checks: DEV_PLAN_CHECKS,
        },
        WorkflowTurn {
            label: "reply_commitment",
            message: "Track this as an open commitment in commitments/open/: reply to the security team about the retry behavior before tomorrow morning.",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "show_commitments_4",
            message: "show commitments",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "passive_signal",
            message: "Slack message from Priya: can you review the OAuth callback edge case this week?",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "show_commitments_5",
            message: "show commitments",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "resolve_ci",
            message: "done with the payments-api CI fix",
            expected_responses: 1,
            checks: DEV_RESOLVED_CI_CHECKS,
        },
        WorkflowTurn {
            label: "resolve_debt",
            message: "resolved the webhook retry helper debt",
            expected_responses: 1,
            checks: DEV_RESOLVED_DEBT_CHECKS,
        },
        WorkflowTurn {
            label: "park_idea_2",
            message: "park this idea in commitments/parked-ideas/: compare event-driven retries versus cron replayers for next quarter",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "show_tech_debt_2",
            message: "show tech debt",
            expected_responses: 1,
            checks: &[],
        },
        WorkflowTurn {
            label: "show_commitments_6",
            message: "show commitments",
            expected_responses: 1,
            checks: &[],
        },
    ];

    // ─────────────────────────────────────────────────────────────────────
    // CEO assistant: setup → meeting capture → workspace verification
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn ceo_full_workflow() {
        run_multi_turn_workflow(
            "ceo_full_workflow",
            "ceo-assistant",
            "ceo",
            CEO_WORKFLOW_TURNS,
            &[
                "ceo-assistant",
                "commitment-digest",
                "decision-capture",
                "delegation-tracker",
                "idea-parking",
            ],
        )
        .await;
    }

    // ─────────────────────────────────────────────────────────────────────
    // Content creator: setup → publication + idea capture → verification
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn content_creator_full_workflow() {
        run_multi_turn_workflow(
            "content_creator_full_workflow",
            "content-creator-assistant",
            "creator",
            CONTENT_CREATOR_WORKFLOW_TURNS,
            &[
                "content-creator-assistant",
                "commitment-digest",
                "decision-capture",
                "idea-parking",
            ],
        )
        .await;
    }

    // ─────────────────────────────────────────────────────────────────────
    // Trader: setup → market signal + decision journal → verification
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn trader_full_workflow() {
        run_multi_turn_workflow(
            "trader_full_workflow",
            "trader-assistant",
            "trader",
            TRADER_WORKFLOW_TURNS,
            &[
                "trader-assistant",
                "commitment-digest",
                "decision-capture",
                "delegation-tracker",
                "idea-parking",
            ],
        )
        .await;
    }

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys or a recorded trace fixture
    async fn developer_full_workflow() {
        if should_run_test("developer_full_workflow") {
            run_multi_turn_workflow(
                "developer_full_workflow",
                "developer-assistant",
                "developer",
                DEVELOPER_WORKFLOW_TURNS,
                &[
                    "developer-assistant",
                    "commitment-digest",
                    "decision-capture",
                    "delegation-tracker",
                    "idea-parking",
                    "tech-debt-tracker",
                    "plan-mode",
                ],
            )
            .await;
        }
    }
}
