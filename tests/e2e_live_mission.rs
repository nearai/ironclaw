//! Live end-to-end test for the mission lifecycle.
//!
//! Walks the agent through:
//!   1. Creating a `daily-news-digest` mission with a manual cadence.
//!   2. Firing it once to generate output.
//!   3. Receiving the mission's notification on the gateway channel.
//!   4. Sending a follow-up question and verifying the mission's output is
//!      part of the assistant conversation context.
//!
//! Run live (records a trace fixture):
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live_mission -- --ignored
//! ```
//!
//! Replay (deterministic, after a fixture has been recorded):
//! ```bash
//! cargo test --features libsql --test e2e_live_mission -- --ignored
//! ```

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod live_mission_tests {
    use std::time::{Duration, Instant};

    use crate::support::live_harness::LiveTestHarnessBuilder;

    /// Channel name to use for the rig — mirrors the real "gateway" channel
    /// so the assistant conversation lookup matches production behavior.
    const CHANNEL: &str = "gateway";

    /// Default user id baked into `TestChannel::new()`. Mission notifications
    /// are scoped to this user, and the assistant conversation lookup uses it.
    const USER_ID: &str = "test-user";

    /// Distinctive marker the agent must use as the mission's name. The
    /// `handle_mission_notification` formatter wraps it as `**[<name>]**`,
    /// which we grep for to identify the mission's notification message.
    const MISSION_NAME: &str = "daily-news-digest";

    /// Wait until at least one captured response (across the rig) contains
    /// `marker`, polling every 500ms until `deadline`. Returns the matching
    /// response text or `None` if the deadline expires.
    async fn wait_for_response_containing(
        rig: &crate::support::test_rig::TestRig,
        marker: &str,
        deadline: Instant,
    ) -> Option<String> {
        loop {
            let responses = rig
                .wait_for_responses(0, Duration::from_millis(0))
                .await;
            if let Some(r) = responses.iter().find(|r| r.content.contains(marker)) {
                return Some(r.content.clone());
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// End-to-end mission lifecycle test.
    ///
    /// IMPORTANT: this test is intentionally strict about the follow-up
    /// behaviour. The agent must reference the digest content when answering
    /// the follow-up — otherwise the mission's output is not actually in the
    /// assistant conversation context, which is the regression we want to
    /// catch.
    /// Best-effort tracing init so `RUST_LOG` works for live debugging.
    /// Multiple tests calling this is fine — `try_init` is idempotent.
    fn init_tracing() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_test_writer()
            .try_init();
    }

    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys (or a recorded trace fixture)
    async fn mission_daily_news_digest_with_followup() {
        init_tracing();
        let harness = LiveTestHarnessBuilder::new("mission_daily_news_digest")
            .with_engine_v2(true)
            .with_max_tool_iterations(40)
            .with_auto_approve_tools(true)
            .with_channel_name(CHANNEL)
            .build()
            .await;

        let rig = harness.rig();

        // ── Turn 1: ask the agent to create + fire the mission ─────────────
        // The mission goal must exercise real tool use — we want this test
        // to fail if missions can't reach tools (web fetch / shell / http)
        // available to the agent. The goal asks the mission thread to fetch
        // a real public source and produce a digest from its actual content.
        let setup_prompt = format!(
            "Create a long-running mission for me using the `mission_create` tool. \
             Use exactly these parameters:\n\
             - name: \"{MISSION_NAME}\"\n\
             - goal: \"Fetch the current Hacker News front page from \
               https://news.ycombinator.com/ (use whichever fetch tool is \
               available — http, web_fetch, or shell with curl) and produce a \
               digest of the top 3 stories. Return the result as a markdown \
               bullet list with exactly 3 bullets, one per story, each bullet \
               starting with '- ' and containing the story title verbatim \
               followed by a one-line summary. After the bullet list, write \
               exactly: 'Mission complete.'\"\n\
             - cadence: \"manual\"\n\
             After the mission is created, immediately call `mission_fire` with its id \
             so it runs once for testing. Then reply to me with: the mission name and \
             a one-sentence confirmation that you fired it. Do not include the mission UUID."
        );

        rig.send_message(&setup_prompt).await;

        // Wait for the agent to finish the setup turn (one assistant response).
        let setup_responses = rig
            .wait_for_responses(1, Duration::from_secs(300))
            .await;
        assert!(
            !setup_responses.is_empty(),
            "expected the setup turn to produce at least one response"
        );

        let setup_text = setup_responses
            .iter()
            .map(|r| r.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        eprintln!(
            "[MissionTest] Setup response: {}",
            setup_text.chars().take(400).collect::<String>()
        );

        // The agent must have actually invoked mission_create + mission_fire.
        let started = rig.tool_calls_started();
        eprintln!("[MissionTest] Tools after setup: {started:?}");
        let used_create = started
            .iter()
            .any(|t| t == "mission_create" || t.starts_with("mission_create("));
        let used_fire = started
            .iter()
            .any(|t| t == "mission_fire" || t.starts_with("mission_fire("));
        assert!(
            used_create,
            "expected agent to call mission_create, got tools: {started:?}"
        );
        assert!(
            used_fire,
            "expected agent to call mission_fire, got tools: {started:?}"
        );

        // The setup turn's user-facing reply should refer to the mission by
        // name (not by raw UUID). This pins the prompt fix that tells the
        // model to reference missions by their name.
        assert!(
            setup_text.contains(MISSION_NAME),
            "expected setup reply to mention mission name '{MISSION_NAME}'; got: {setup_text}"
        );

        // ── Wait for the mission's notification to land on the channel ─────
        // `handle_mission_notification` prefixes the message with `**[name]**`.
        // We give the mission a generous budget — fetching HN, parsing the
        // page, and producing a 3-bullet digest can take many minutes
        // against a slower model with a multi-step CodeAct loop.
        let mission_marker = format!("**[{MISSION_NAME}]**");
        let mission_deadline = Instant::now() + Duration::from_secs(900);
        let mission_text = match wait_for_response_containing(
            rig,
            &mission_marker,
            mission_deadline,
        )
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
                panic!(
                    "mission notification with marker '{mission_marker}' did not arrive within \
                     15 minutes. Captured responses so far: {captured:#?}"
                );
            }
        };
        eprintln!(
            "[MissionTest] Mission notification: {}",
            mission_text.chars().take(400).collect::<String>()
        );

        // ── Verify the mission's output was persisted to the assistant
        //    conversation in the v1 DB. This is the surface the gateway's
        //    history API reads from, and it's the channel-keyed assistant
        //    conversation that follow-up messages route into.
        let db = rig.database();
        let conv_id = db
            .get_or_create_assistant_conversation(USER_ID, CHANNEL)
            .await
            .expect("get_or_create_assistant_conversation should succeed");
        let conv_messages = db
            .list_conversation_messages(conv_id)
            .await
            .expect("list_conversation_messages should succeed");
        let assistant_contents: Vec<String> = conv_messages
            .iter()
            .filter(|m| m.role == "assistant")
            .map(|m| m.content.clone())
            .collect();
        let mission_in_db = assistant_contents
            .iter()
            .any(|c| c.contains(&mission_marker));
        assert!(
            mission_in_db,
            "mission notification should be persisted to the gateway assistant conversation. \
             Assistant messages found: {assistant_contents:#?}"
        );

        // ── Turn 2: send a follow-up that only makes sense if the mission's
        //    output is part of the assistant conversation context.
        let baseline = rig
            .wait_for_responses(0, Duration::from_millis(0))
            .await
            .len();
        let followup = "Looking at the news digest you just sent me, pick the single most \
            interesting headline from it and explain in 2-3 sentences why it matters. \
            Quote the headline verbatim and do NOT ask me to provide it — use the digest \
            you already delivered.";
        rig.send_message(followup).await;

        let after_followup = rig
            .wait_for_responses(baseline + 1, Duration::from_secs(300))
            .await;
        assert!(
            after_followup.len() > baseline,
            "expected at least one new response after the follow-up; \
             baseline={baseline}, total={}",
            after_followup.len()
        );
        let followup_text = after_followup
            .iter()
            .skip(baseline)
            .map(|r| r.content.clone())
            .collect::<Vec<_>>()
            .join("\n");
        eprintln!(
            "[MissionTest] Follow-up response: {}",
            followup_text.chars().take(600).collect::<String>()
        );

        // The follow-up reply must NOT be a refusal/clarification. A simple
        // structural check: the answer must not contain "no record" / "haven't
        // sent" / "did not receive" style phrases, and it must be of
        // non-trivial length (a real explanation).
        let lower = followup_text.to_lowercase();
        let refusal_markers = [
            "haven't sent",
            "have not sent",
            "no digest",
            "no record",
            "did not receive",
            "didn't receive",
            "i don't see",
            "no previous digest",
            "could you share",
            "could you provide",
            "please share the digest",
            "please provide the digest",
        ];
        for marker in refusal_markers {
            assert!(
                !lower.contains(marker),
                "follow-up reply should not be a refusal/clarification (matched '{marker}'). \
                 The mission's output is missing from the assistant conversation context. \
                 Reply was: {followup_text}"
            );
        }
        assert!(
            followup_text.len() > 80,
            "expected a substantive follow-up explanation; got short reply: {followup_text}"
        );

        // Strong semantic check via LLM judge (live mode only).
        let criteria = "The response is a substantive answer that quotes a specific headline \
            from a previously delivered news digest and explains in a few sentences why that \
            headline matters. It does NOT ask the user to provide the digest, claim no digest \
            was sent, or refuse to answer.";
        if let Some(verdict) = harness
            .judge(std::slice::from_ref(&followup_text), criteria)
            .await
        {
            assert!(
                verdict.pass,
                "LLM judge rejected the follow-up reply: {}",
                verdict.reasoning
            );
        }

        let all_text: Vec<String> = after_followup.iter().map(|r| r.content.clone()).collect();
        harness.finish(&setup_prompt, &all_text).await;
    }
}
