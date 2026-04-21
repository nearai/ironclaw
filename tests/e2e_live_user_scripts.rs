//! Live-only smoke tests derived from external IronClaw user test scripts.
//!
//! These scenarios intentionally skip replay fixtures and LLM trace recording:
//! they exercise real OAuth / routine-creation flows that would otherwise
//! capture personal calendar, inbox, spreadsheet, or Telegram data.
//!
//! Run live:
//! ```bash
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live_user_scripts -- --ignored --nocapture
//! ```

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod live_user_script_tests {
    use std::time::{Duration, Instant};

    use crate::support::live_harness::{LiveTestHarness, LiveTestHarnessBuilder, TestMode};
    use ironclaw::agent::routine::{Routine, Trigger};

    const USER_ID: &str = "test-user";
    const TELEGRAM_SECRET: &str = "telegram_bot_token";
    const GOOGLE_SECRETS: [&str; 3] = [
        "google_oauth_token",
        "google_oauth_token_refresh_token",
        "google_oauth_token_scopes",
    ];

    fn init_tracing() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
            )
            .with_test_writer()
            .try_init();
    }

    async fn wait_for_routine_named(
        harness: &LiveTestHarness,
        routine_name: &str,
        timeout: Duration,
    ) -> Option<Routine> {
        let deadline = Instant::now() + timeout;
        loop {
            let routines = harness
                .rig()
                .database()
                .list_routines(USER_ID)
                .await
                .expect("list_routines should succeed");
            if let Some(routine) = routines.into_iter().find(|r| r.name == routine_name) {
                return Some(routine);
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn build_live_only_harness(
        test_name: &str,
        max_tool_iterations: usize,
        secrets: &[&str],
    ) -> Option<LiveTestHarness> {
        let mut builder = LiveTestHarnessBuilder::new(test_name)
            .with_engine_v2(true)
            .with_auto_approve_tools(true)
            .with_max_tool_iterations(max_tool_iterations)
            .with_no_trace_recording();
        if !secrets.is_empty() {
            builder = builder.with_secrets(secrets.iter().copied());
        }
        let harness = builder.build().await;
        if harness.mode() != TestMode::Live {
            eprintln!(
                "[LiveUserScripts] {test_name}: live-only scenario — skipping outside IRONCLAW_LIVE_TEST=1"
            );
            return None;
        }

        Some(harness)
    }

    async fn send_prompt_and_capture(
        harness: &LiveTestHarness,
        prompt: &str,
        timeout: Duration,
    ) -> (Vec<String>, Vec<String>) {
        let rig = harness.rig();
        rig.send_message(prompt).await;
        let responses = rig.wait_for_responses(1, timeout).await;
        assert!(
            !responses.is_empty(),
            "Expected at least one response for prompt: {prompt}"
        );
        let text: Vec<String> = responses.iter().map(|r| r.content.clone()).collect();
        let tools = rig.tool_calls_started();
        eprintln!("[LiveUserScripts] Prompt: {prompt}");
        eprintln!("[LiveUserScripts] Tools: {tools:?}");
        eprintln!(
            "[LiveUserScripts] Response preview: {}",
            text.join("\n").chars().take(800).collect::<String>()
        );
        (text, tools)
    }

    fn assert_text_mentions(joined_lower: &str, needles: &[&str], label: &str) {
        assert!(
            needles
                .iter()
                .any(|needle| joined_lower.contains(&needle.to_lowercase())),
            "Expected {label} to mention one of {needles:?}, got: {joined_lower}"
        );
    }

    fn assert_tool_used(tools: &[String], prefixes: &[&str], label: &str) {
        let matched = tools.iter().any(|tool| {
            prefixes
                .iter()
                .any(|prefix| tool == prefix || tool.starts_with(&format!("{prefix}(")))
        });
        assert!(
            matched,
            "Expected {label} to use one of {prefixes:?}, got tools={tools:?}"
        );
    }

    fn scheduler_creation_kind(tools: &[String]) -> Option<&'static str> {
        if tools
            .iter()
            .any(|tool| tool == "routine_create" || tool.starts_with("routine_create("))
        {
            return Some("routine");
        }
        if tools
            .iter()
            .any(|tool| tool == "mission_create" || tool.starts_with("mission_create("))
        {
            return Some("mission");
        }
        None
    }

    async fn assert_scheduler_created(
        harness: &LiveTestHarness,
        scheduler_name: &str,
        tools: &[String],
        text: &[String],
        require_cron: bool,
    ) -> Option<Routine> {
        let kind = scheduler_creation_kind(tools).unwrap_or_else(|| {
            panic!(
                "Expected {scheduler_name} to use routine_create or mission_create, got tools={tools:?} response={text:?}"
            )
        });
        if kind == "routine" {
            let routine = wait_for_routine_named(harness, scheduler_name, Duration::from_secs(30))
                .await
                .unwrap_or_else(|| {
                    panic!(
                        "Routine '{scheduler_name}' was not created. Tools={tools:?} response={text:?}"
                    )
                });
            assert!(routine.enabled, "Expected {scheduler_name} to be enabled");
            if require_cron {
                assert_cron_trigger(&routine, scheduler_name);
            }
            Some(routine)
        } else {
            None
        }
    }

    fn assert_cron_trigger(routine: &Routine, label: &str) {
        match &routine.trigger {
            Trigger::Cron { schedule, .. } => {
                assert!(
                    !schedule.trim().is_empty(),
                    "Expected {label} cron schedule to be non-empty"
                );
            }
            other => panic!("Expected {label} to create a cron routine, got trigger={other:?}"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn script_periodic_telegram_reminder() {
        init_tracing();
        let Some(harness) =
            build_live_only_harness("script_periodic_telegram_reminder", 60, &[TELEGRAM_SECRET])
                .await
        else {
            return;
        };

        let routine_name = "live-dog-walk-reminder";
        let prompt = format!(
            "Create a cron routine named '{routine_name}' that reminds me to take my dog for a walk every 2 minutes. Configure it to notify me on the telegram channel. Reply with the routine name and schedule."
        );
        let (text, tools) =
            send_prompt_and_capture(&harness, &prompt, Duration::from_secs(300)).await;
        let joined = text.join("\n").to_lowercase();
        assert_tool_used(&tools, &["routine_create", "mission_create"], routine_name);
        assert_text_mentions(
            &joined,
            &[routine_name, "telegram", "2 minute", "cron"],
            routine_name,
        );

        let _routine = assert_scheduler_created(&harness, routine_name, &tools, &text, true).await;

        harness.finish(&prompt, &text).await;
    }

    #[tokio::test]
    #[ignore]
    async fn script_hacker_news_show_hn_monitor() {
        init_tracing();
        let Some(harness) =
            build_live_only_harness("script_hacker_news_show_hn_monitor", 80, &[TELEGRAM_SECRET])
                .await
        else {
            return;
        };

        let routine_name = "live-show-hn-monitor";
        let prompt = format!(
            "Create a cron routine named '{routine_name}' that checks Hacker News every hour for Show HN posts, sends a summary to telegram, and runs the first check immediately after creating it. Reply with the routine name and whether you fired it."
        );
        let (text, tools) =
            send_prompt_and_capture(&harness, &prompt, Duration::from_secs(420)).await;
        let joined = text.join("\n").to_lowercase();
        assert_tool_used(&tools, &["routine_create", "mission_create"], routine_name);
        assert_text_mentions(
            &joined,
            &[routine_name, "show hn", "hacker news", "telegram"],
            routine_name,
        );

        let _routine = assert_scheduler_created(&harness, routine_name, &tools, &text, true).await;

        harness.finish(&prompt, &text).await;
    }

    #[tokio::test]
    #[ignore]
    async fn script_calendar_prep_assistant() {
        init_tracing();
        let mut secrets = Vec::from(GOOGLE_SECRETS);
        secrets.push(TELEGRAM_SECRET);
        let Some(harness) =
            build_live_only_harness("script_calendar_prep_assistant", 80, &secrets).await
        else {
            return;
        };

        let routine_name = "live-calendar-prep-assistant";
        let prompt = format!(
            "Create a routine named '{routine_name}' that 10 minutes before each Google Calendar meeting sends me a Telegram prep summary with attendee company background and recent news. Reply with the routine name and trigger type."
        );
        let (text, tools) =
            send_prompt_and_capture(&harness, &prompt, Duration::from_secs(420)).await;
        let joined = text.join("\n").to_lowercase();
        assert_tool_used(&tools, &["routine_create", "mission_create"], routine_name);
        assert_text_mentions(
            &joined,
            &[routine_name, "calendar", "telegram", "10 minutes"],
            routine_name,
        );

        if let Some(routine) =
            assert_scheduler_created(&harness, routine_name, &tools, &text, false).await
        {
            match &routine.trigger {
                Trigger::Cron { .. } | Trigger::Event { .. } => {}
                other => panic!(
                    "Expected {routine_name} trigger to be cron or event-based, got {other:?}"
                ),
            }
        }

        harness.finish(&prompt, &text).await;
    }

    #[tokio::test]
    #[ignore]
    async fn script_telegram_bug_logger() {
        init_tracing();
        let mut secrets = Vec::from(GOOGLE_SECRETS);
        secrets.push(TELEGRAM_SECRET);
        let Some(harness) =
            build_live_only_harness("script_telegram_bug_logger", 80, &secrets).await
        else {
            return;
        };

        let routine_name = "live-telegram-bug-to-sheets";
        let prompt = format!(
            "Create a new Google Sheet called 'Live Bug Log' if needed, then create a cron routine named '{routine_name}' that every 2 minutes appends any Telegram message starting with 'bug:' into that sheet with columns timestamp, message, source. Strip the 'bug:' prefix in the stored message. Reply with the routine name and the sheet name."
        );
        let (text, tools) =
            send_prompt_and_capture(&harness, &prompt, Duration::from_secs(420)).await;
        let joined = text.join("\n").to_lowercase();
        assert_tool_used(&tools, &["routine_create", "mission_create"], routine_name);
        assert_text_mentions(
            &joined,
            &[routine_name, "live bug log", "telegram", "sheet"],
            routine_name,
        );

        let _routine = assert_scheduler_created(&harness, routine_name, &tools, &text, true).await;

        harness.finish(&prompt, &text).await;
    }

    #[tokio::test]
    #[ignore]
    async fn script_email_crm_inbound_tracker() {
        init_tracing();
        let Some(harness) =
            build_live_only_harness("script_email_crm_inbound_tracker", 80, &GOOGLE_SECRETS).await
        else {
            return;
        };

        let routine_name = "live-email-crm-tracker";
        let prompt = format!(
            "Create a Google Sheet called 'Inbound CRM' if needed, then create an hourly cron routine named '{routine_name}' that reads Gmail for inbound sales leads and appends rows with columns Company, Contact Name, Email, Status, Notes, Next Action. Reply with the routine name and the sheet name."
        );
        let (text, tools) =
            send_prompt_and_capture(&harness, &prompt, Duration::from_secs(420)).await;
        let joined = text.join("\n").to_lowercase();
        assert_tool_used(&tools, &["routine_create", "mission_create"], routine_name);
        assert_text_mentions(
            &joined,
            &[routine_name, "inbound crm", "gmail", "sheet"],
            routine_name,
        );

        let _routine = assert_scheduler_created(&harness, routine_name, &tools, &text, true).await;

        harness.finish(&prompt, &text).await;
    }
}
