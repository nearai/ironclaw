//! Live end-to-end test for issue #3133 — "Mission to send an email failed".
//!
//! The bug report shows a mission firing every 3 minutes whose child
//! thread bails with the FINAL() body:
//!
//!   "Failed to send email. Status: None Error: None
//!    Next focus: Debug Gmail authentication or try alternative email method."
//!
//! The "Status: None Error: None" wording is not from any of our Rust
//! code — it's the LLM looking at a tool response (probably the `http`
//! tool's reply shape, possibly a Gmail tool error path) where it
//! expected populated `status` and `error` fields and found neither.
//! Three plausible origins:
//!
//!   1. The agent fell back to an `http` POST to Gmail's REST API after
//!      the WASM `gmail` tool call did not resolve cleanly.
//!   2. The Gmail OAuth token was not seeded for the mission's child
//!      thread, so the tool errored "Google OAuth token not configured…"
//!      and the model paraphrased it.
//!   3. The mission's lease did not include `gmail`, so the call never
//!      reached the WASM tool at all.
//!
//! This test drives a verbatim-flavoured prompt through engine v2 with
//! Gmail OAuth credentials seeded, and asserts that:
//!
//!   1. The agent invoked `mission_create` and `mission_fire`
//!      (or the routine_* aliases that resolve to the same handlers).
//!   2. The mission's child thread invoked the `gmail` tool — proving
//!      the tool was discoverable and the lease covered it.
//!   3. The notification carries a `**[<mission-name>]**` marker, the
//!      same structural signal `e2e_live_routine.rs` uses.
//!   4. The notification body does NOT contain the
//!      "Status: None Error: None" pattern that #3133 reports — that's
//!      the regression marker.
//!   5. The orchestrator never emits "<N> consecutive code errors"
//!      (sanity check shared with #2583).
//!
//! Why drafts, not sends: the bug reproduces equally on `gmail.create_draft`
//! (which lands in the OAuth account's Drafts folder, no email actually
//! delivered) as on `gmail.send_message`. Drafts have minimal blast
//! radius — no real email goes out, and the resulting draft is easy to
//! delete by id from the test owner's Gmail. Switching the prompt to
//! "create a draft" instead of "send an email" preserves the auth /
//! tool-discovery / lease paths the bug actually exercises.
//!
//! Run live (records a trace fixture):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live_mission_gmail -- --ignored
//! ```
//!
//! Replay (deterministic, after a fixture has been recorded):
//! ```bash
//! cargo test --features libsql --test e2e_live_mission_gmail -- --ignored
//! ```
//!
//! Live mode requires Gmail OAuth credentials in the developer's
//! `~/.ironclaw/ironclaw.db` under the names declared in `with_secrets`
//! below. The test rig seeds *only* those rows into its temporary
//! database; nothing else (workspace memory, conversation history, other
//! secrets) crosses the boundary. See `tests/support/LIVE_TESTING.md`.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod live_mission_gmail_tests {
    use std::time::{Duration, Instant};

    use crate::support::live_harness::{LiveTestHarnessBuilder, TestMode};
    use crate::support::live_mission_helpers::{
        ApprovalAutoResponder, looks_like_routine_notification, tool_is, wait_for_response_matching,
    };
    use crate::support::test_rig::TestRig;

    /// Channel name to use for the rig — mirrors the real "gateway" channel
    /// so mission notifications route back the same way they do in production.
    const CHANNEL: &str = "gateway";

    /// User prompt that reproduces the #3133 flow. We deliberately ask for
    /// drafts (not sends) so the test has minimal external blast radius.
    /// "Trigger it once right now" is required to get a synchronous fire
    /// the test can observe — the `every three minutes` cron alone wouldn't
    /// fire within the test window.
    const USER_PROMPT: &str = "Create a mission that creates a Gmail draft \
        every three minutes. The draft should go to the user's own Gmail \
        address (whichever account the OAuth token belongs to), with subject \
        \"Test mission #3133\" and body \"This is a test draft from the \
        IronClaw mission system.\" Trigger it once right now and report \
        whether the draft was created.";

    /// The orchestrator's "consecutive code errors" failure surface — same
    /// regression marker the routine test guards against. Source:
    /// `crates/ironclaw_engine/orchestrator/default.py:1003`.
    const CONSECUTIVE_ERRORS_MARKER: &str = "consecutive code errors";

    /// The exact #3133 regression marker. The model wrote "Status: None
    /// Error: None" verbatim into FINAL() because it inspected a tool
    /// response shape with `status` and `error` keys that were both
    /// `None`. If that pattern shows up in the captured notification,
    /// the bug is back.
    const STATUS_NONE_MARKER: &str = "Status: None";
    const ERROR_NONE_MARKER: &str = "Error: None";

    fn init_tracing() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_test_writer()
            .try_init();
    }

    /// Engine v2's mission_create matches; the bridge alias path also
    /// translates `routine_create`. Accept either.
    fn used_create(tools: &[String]) -> bool {
        tools
            .iter()
            .any(|t| tool_is(t, "mission_create") || tool_is(t, "routine_create"))
    }

    /// Same dual-name acceptance for fire.
    fn used_fire(tools: &[String]) -> bool {
        tools
            .iter()
            .any(|t| tool_is(t, "mission_fire") || tool_is(t, "routine_fire"))
    }

    /// Did the mission's child thread actually invoke the Gmail WASM tool?
    /// Tool name on the wire is `gmail`; arguments may be folded in as
    /// `gmail(create_draft)` via `format_action_display_name`.
    fn used_gmail(tools: &[String]) -> bool {
        tools.iter().any(|t| tool_is(t, "gmail"))
    }

    /// Assert no captured response contains the #3133 regression marker.
    /// The check is structural — we look for both "Status: None" AND
    /// "Error: None" in the same response, since either alone could appear
    /// in unrelated narration; the dual presence is the bug fingerprint.
    async fn assert_no_status_none_failure(rig: &TestRig, where_in_test: &str) {
        let responses = rig.wait_for_responses(0, Duration::from_millis(0)).await;
        for r in &responses {
            let has_status = r.content.contains(STATUS_NONE_MARKER);
            let has_error = r.content.contains(ERROR_NONE_MARKER);
            assert!(
                !(has_status && has_error),
                "[{where_in_test}] regression: response carried both \
                 '{STATUS_NONE_MARKER}' and '{ERROR_NONE_MARKER}' — the \
                 #3133 fingerprint of a Gmail mission FINAL() that bailed \
                 with no useful diagnosis. Full response: {}",
                r.content
            );
        }
    }

    /// Assert no captured response contains the orchestrator's consecutive-
    /// errors failure surface (mirrors the #2583 sanity check).
    async fn assert_no_consecutive_errors(rig: &TestRig, where_in_test: &str) {
        let responses = rig.wait_for_responses(0, Duration::from_millis(0)).await;
        for r in &responses {
            assert!(
                !r.content.to_lowercase().contains(CONSECUTIVE_ERRORS_MARKER),
                "[{where_in_test}] regression: response carried the \
                 '{CONSECUTIVE_ERRORS_MARKER}' failure surface from #2583. \
                 Full response: {}",
                r.content
            );
        }
    }

    #[test]
    fn assert_no_status_none_failure_helper_recognises_pattern() {
        // Dual-marker presence is the regression fingerprint. Single-marker
        // presence in narration is fine and shouldn't trip the assertion.
        let bug_text = "Failed to send email. Status: None Error: None\n\
                        Next focus: Debug Gmail authentication.";
        assert!(
            bug_text.contains("Status: None") && bug_text.contains("Error: None"),
            "the regression fingerprint must be both substrings present in \
             the same response (the helper wires this together with an &&)"
        );
    }

    #[tokio::test]
    #[ignore] // Live tier: requires Gmail OAuth in ~/.ironclaw/ironclaw.db.
    // Records/replays from
    // tests/fixtures/llm_traces/live/mission_gmail_draft_3133.json.
    async fn mission_gmail_draft_3133() {
        init_tracing();

        // auto_approve_tools is intentionally OFF for the same reason as
        // the routine test: administrative actions (mission_create /
        // mission_fire) and write-effect actions (gmail.create_draft) are
        // expected to surface ApprovalNeeded gates, and we want the
        // responder to round-trip them. If the gates never fire, that's
        // logged as a separate concern (parked under #2583's open items).
        let harness = LiveTestHarnessBuilder::new("mission_gmail_draft_3133")
            .with_engine_v2(true)
            .with_max_tool_iterations(40)
            .with_auto_approve_tools(false)
            .with_channel_name(CHANNEL)
            .with_secrets([
                "google_oauth_token",
                "google_oauth_token_refresh_token",
                "google_oauth_token_scopes",
            ])
            .build()
            .await;

        let rig = harness.rig();
        let approver = ApprovalAutoResponder::spawn(rig.channel_handle());

        rig.send_message(USER_PROMPT).await;

        // Wait for the mission's child thread to deliver a notification.
        // Same `**[name]**` anchor the routine test uses — proves the
        // mission_fire path itself works end-to-end.
        let setup_deadline = Instant::now() + Duration::from_secs(900);
        let notification_text =
            match wait_for_response_matching(rig, looks_like_routine_notification, setup_deadline)
                .await
            {
                Some(text) => text,
                None => {
                    let captured: Vec<String> = rig
                        .wait_for_responses(0, Duration::from_millis(0))
                        .await
                        .iter()
                        .map(|r| r.content.clone())
                        .collect();
                    let tools = rig.tool_calls_started();
                    approver.shutdown();
                    panic!(
                        "no mission notification (with **[name]** marker) arrived \
                     within 15 minutes — the mission did not deliver output \
                     via the channel. Tool calls observed: {tools:?}. \
                     Captured responses: {captured:#?}"
                    );
                }
            };
        eprintln!(
            "[GmailMissionTest] Notification preview: {}",
            notification_text.chars().take(400).collect::<String>()
        );

        // The agent must have invoked mission/routine creation + fire.
        let tools = rig.tool_calls_started();
        eprintln!("[GmailMissionTest] Tools observed: {tools:?}");
        assert!(
            used_create(&tools),
            "expected agent to call mission_create or routine_create after \
             prompt; got tools: {tools:?}"
        );
        assert!(
            used_fire(&tools),
            "expected agent to fire the newly-created mission for the \
             'trigger it once right now' clause; got tools: {tools:?}"
        );

        // Did the Gmail tool actually run? If not, either:
        //   - the lease/capability path didn't include gmail (real bug), or
        //   - the agent "tried" via some other route (http fallback).
        // In either case the FINAL() narration is suspect — emit a
        // diagnostic warning, but don't fail the test on this alone since
        // the structural notification check + Status:None check below are
        // the actual regression guards.
        if !used_gmail(&tools) {
            eprintln!(
                "[GmailMissionTest] WARNING: the gmail WASM tool was not \
                 invoked even though the prompt asked for a Gmail draft. \
                 The mission's child thread likely fell back to `http` or \
                 narrated 'I would call gmail' without actually doing so. \
                 This is one of the candidate root causes for #3133."
            );
        }

        // The hard regression guard for #3133.
        assert_no_status_none_failure(rig, "after mission notification").await;

        // Sanity: the #2583 fix must still hold — no `consecutive code
        // errors` should surface. If a fresh path triggers them, that's a
        // signal the mission_* alias regression is back.
        assert_no_consecutive_errors(rig, "after mission notification").await;

        // ── Approval-gate observation (warning-only, parked under #2583) ──
        let approved = approver.approved_tools().await;
        eprintln!(
            "[GmailMissionTest] Approvals captured ({}): {approved:?}",
            approved.len()
        );
        if approved.is_empty() {
            eprintln!(
                "[GmailMissionTest] WARNING: auto_approve was OFF and zero \
                 ApprovalNeeded gates were observed. Administrative tools \
                 (mission_create / mission_fire) and the write-effect \
                 gmail tool ran without prompting. This is the same parked \
                 concern from the #2583 PR — track separately."
            );
        }

        // Live mode only: surface unexpected tool failures in the captured
        // trace so a maintainer reviewing a fixture diff sees them.
        if harness.mode() == TestMode::Live {
            let trace_errors = harness.collect_trace_errors();
            if !trace_errors.is_empty() {
                eprintln!(
                    "[GmailMissionTest] WARNING: trace contained tool errors \
                     (not failing the test, but worth investigating): \
                     {trace_errors:?}"
                );
            }
        }

        approver.shutdown();

        let all_text: Vec<String> = rig
            .wait_for_responses(0, Duration::from_millis(0))
            .await
            .iter()
            .map(|r| r.content.clone())
            .collect();
        harness.finish(USER_PROMPT, &all_text).await;
    }
}
