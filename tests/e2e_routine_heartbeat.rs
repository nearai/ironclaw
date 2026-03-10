//! E2E tests: routine engine and heartbeat (#575).
//!
//! These tests construct RoutineEngine and HeartbeatRunner directly
//! with a TraceLlm and libSQL database, bypassing the full TestRig.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use uuid::Uuid;

    use ironclaw::agent::routine::{
        NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger,
    };
    use ironclaw::agent::routine_engine::RoutineEngine;
    use ironclaw::agent::{HeartbeatConfig, HeartbeatRunner};
    use ironclaw::channels::IncomingMessage;
    use ironclaw::config::{RoutineConfig, SafetyConfig};
    use ironclaw::db::Database;
    use ironclaw::safety::SafetyLayer;
    use ironclaw::tools::ToolRegistry;
    use ironclaw::workspace::Workspace;
    use ironclaw::workspace::hygiene::HygieneConfig;

    use crate::support::trace_llm::{LlmTrace, TraceLlm, TraceResponse, TraceStep};

    /// Create a temp libSQL database with migrations applied.
    async fn create_test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("LibSqlBackend");
        backend.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::new(backend);
        (db, temp_dir)
    }

    /// Create a workspace backed by the test database.
    fn create_workspace(db: &Arc<dyn Database>) -> Arc<Workspace> {
        Arc::new(Workspace::new_with_db("default", db.clone()))
    }

    /// Helper to insert a routine directly into the database.
    fn make_routine(name: &str, trigger: Trigger, prompt: &str) -> Routine {
        Routine {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: format!("Test routine: {name}"),
            user_id: "default".to_string(),
            enabled: true,
            trigger,
            action: RoutineAction::Lightweight {
                prompt: prompt.to_string(),
                context_paths: vec![],
                max_tokens: 1000,
            },
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(0),
                max_concurrent: 5,
                dedup_window: None,
            },
            notify: NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: cron_routine_fires
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn cron_routine_fires() {
        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        // Create a TraceLlm that responds with ROUTINE_OK.
        let trace = LlmTrace::single_turn(
            "test-cron-fire",
            "check",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "ROUTINE_OK".to_string(),
                    input_tokens: 50,
                    output_tokens: 5,
                },
                expected_tool_results: vec![],
            }],
        );
        let llm = Arc::new(TraceLlm::from_trace(trace));

        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel(16);

        // Create minimal ToolRegistry and SafetyLayer for test.
        let tools = Arc::new(ToolRegistry::new());
        let safety_config = SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        };
        let safety = Arc::new(SafetyLayer::new(&safety_config));

        let engine = Arc::new(RoutineEngine::new(
            RoutineConfig::default(),
            db.clone(),
            llm,
            ws,
            notify_tx,
            None,
            tools,
            safety,
        ));

        // Insert a cron routine with next_fire_at in the past.
        let mut routine = make_routine(
            "cron-test",
            Trigger::Cron {
                schedule: "* * * * *".to_string(),
                timezone: None,
            },
            "Check system status.",
        );
        routine.next_fire_at = Some(Utc::now() - chrono::Duration::minutes(5));
        db.create_routine(&routine).await.expect("create_routine");

        // Fire cron triggers.
        engine.check_cron_triggers().await;

        // Give the spawned task time to execute.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Verify a run was recorded.
        let runs = db
            .list_routine_runs(routine.id, 10)
            .await
            .expect("list_routine_runs");
        assert!(
            !runs.is_empty(),
            "Expected at least one routine run after cron trigger"
        );

        // Notification may or may not be sent depending on config;
        // just verify no panic occurred. Drain the channel.
        let _ = notify_rx.try_recv();
    }

    // -----------------------------------------------------------------------
    // Test 2: event_trigger_matches
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn event_trigger_matches() {
        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        let trace = LlmTrace::single_turn(
            "test-event-match",
            "deploy",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "Deployment detected".to_string(),
                    input_tokens: 50,
                    output_tokens: 10,
                },
                expected_tool_results: vec![],
            }],
        );
        let llm = Arc::new(TraceLlm::from_trace(trace));
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::channel(16);

        // Create minimal ToolRegistry and SafetyLayer for test.
        let tools = Arc::new(ToolRegistry::new());
        let safety_config = SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        };
        let safety = Arc::new(SafetyLayer::new(&safety_config));

        let engine = Arc::new(RoutineEngine::new(
            RoutineConfig::default(),
            db.clone(),
            llm,
            ws,
            notify_tx,
            None,
            tools,
            safety,
        ));

        // Insert an event routine matching "deploy.*production".
        let routine = make_routine(
            "deploy-watcher",
            Trigger::Event {
                channel: None,
                pattern: "deploy.*production".to_string(),
            },
            "Report on deployment.",
        );
        db.create_routine(&routine).await.expect("create_routine");

        // Refresh the event cache so the engine knows about the routine.
        engine.refresh_event_cache().await;

        // Positive match: message containing "deploy to production".
        let matching_msg = IncomingMessage {
            id: Uuid::new_v4(),
            channel: "test".to_string(),
            user_id: "default".to_string(),
            user_name: None,
            content: "deploy to production now".to_string(),
            thread_id: None,
            received_at: Utc::now(),
            metadata: serde_json::json!({}),
            timezone: None,
            attachments: Vec::new(),
        };
        let fired = engine.check_event_triggers(&matching_msg).await;
        assert!(
            fired >= 1,
            "Expected >= 1 routine fired on match, got {fired}"
        );

        // Give spawn time.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Negative match: message that doesn't match.
        let non_matching_msg = IncomingMessage {
            id: Uuid::new_v4(),
            channel: "test".to_string(),
            user_id: "default".to_string(),
            user_name: None,
            content: "check the staging environment".to_string(),
            thread_id: None,
            received_at: Utc::now(),
            metadata: serde_json::json!({}),
            timezone: None,
            attachments: Vec::new(),
        };
        let fired_neg = engine.check_event_triggers(&non_matching_msg).await;
        assert_eq!(fired_neg, 0, "Expected 0 routines fired on non-match");
    }

    // -----------------------------------------------------------------------
    // Test 3: system_event_trigger_matches_and_filters
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn system_event_trigger_matches_and_filters() {
        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        let trace = LlmTrace::single_turn(
            "test-system-event-match",
            "event",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "System event handled".to_string(),
                    input_tokens: 40,
                    output_tokens: 8,
                },
                expected_tool_results: vec![],
            }],
        );
        let llm = Arc::new(TraceLlm::from_trace(trace));
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::channel(16);

        // Create minimal ToolRegistry and SafetyLayer for test.
        let tools = Arc::new(ToolRegistry::new());
        let safety_config = SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        };
        let safety = Arc::new(SafetyLayer::new(&safety_config));

        let engine = Arc::new(RoutineEngine::new(
            RoutineConfig::default(),
            db.clone(),
            llm,
            ws,
            notify_tx,
            None,
            tools,
            safety,
        ));

        let mut filters = std::collections::HashMap::new();
        filters.insert("repository".to_string(), "nearai/ironclaw".to_string());

        let routine = make_routine(
            "github-issue-opened",
            Trigger::SystemEvent {
                source: "github".to_string(),
                event_type: "issue.opened".to_string(),
                filters,
            },
            "Summarize the issue and propose an implementation plan.",
        );
        db.create_routine(&routine).await.expect("create_routine");
        engine.refresh_event_cache().await;

        // Matching event should fire.
        let fired = engine
            .emit_system_event(
                "github",
                "issue.opened",
                &serde_json::json!({
                    "repository": "nearai/ironclaw",
                    "issue_number": 42
                }),
                Some("default"),
            )
            .await;
        assert_eq!(fired, 1, "Expected one routine to fire for matching event");

        tokio::time::sleep(Duration::from_millis(300)).await;

        let runs = db
            .list_routine_runs(routine.id, 10)
            .await
            .expect("list runs");
        assert!(
            !runs.is_empty(),
            "Expected run history after matching event"
        );

        // Wrong event type should not fire.
        let fired_wrong_type = engine
            .emit_system_event(
                "github",
                "issue.closed",
                &serde_json::json!({"repository": "nearai/ironclaw"}),
                Some("default"),
            )
            .await;
        assert_eq!(
            fired_wrong_type, 0,
            "Expected no routine for wrong event type"
        );

        // Wrong filter value should not fire.
        let fired_wrong_filter = engine
            .emit_system_event(
                "github",
                "issue.opened",
                &serde_json::json!({"repository": "other/repo"}),
                Some("default"),
            )
            .await;
        assert_eq!(
            fired_wrong_filter, 0,
            "Expected no routine for filter mismatch"
        );

        // Case-insensitive source/event_type should still match.
        let fired_case = engine
            .emit_system_event(
                "GitHub",
                "Issue.Opened",
                &serde_json::json!({
                    "repository": "nearai/ironclaw",
                    "issue_number": 99
                }),
                Some("default"),
            )
            .await;
        assert_eq!(
            fired_case, 1,
            "Expected case-insensitive match on source/event_type"
        );

        // Case-insensitive filter values should match.
        let fired_filter_case = engine
            .emit_system_event(
                "github",
                "issue.opened",
                &serde_json::json!({"repository": "NearAI/IronClaw"}),
                Some("default"),
            )
            .await;
        assert_eq!(
            fired_filter_case, 1,
            "Expected case-insensitive match on filter values"
        );
    }

    // -----------------------------------------------------------------------
    // Test 4: routine_cooldown
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn routine_cooldown() {
        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        // Need two LLM responses (one for the first fire).
        let trace = LlmTrace::single_turn(
            "test-cooldown",
            "check",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "ROUTINE_OK".to_string(),
                    input_tokens: 50,
                    output_tokens: 5,
                },
                expected_tool_results: vec![],
            }],
        );
        let llm = Arc::new(TraceLlm::from_trace(trace));
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::channel(16);

        // Create minimal ToolRegistry and SafetyLayer for test.
        let tools = Arc::new(ToolRegistry::new());
        let safety_config = SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        };
        let safety = Arc::new(SafetyLayer::new(&safety_config));

        let engine = Arc::new(RoutineEngine::new(
            RoutineConfig::default(),
            db.clone(),
            llm,
            ws,
            notify_tx,
            None,
            tools,
            safety,
        ));

        // Insert an event routine with 1-hour cooldown.
        let mut routine = make_routine(
            "cooldown-test",
            Trigger::Event {
                channel: None,
                pattern: "test-cooldown".to_string(),
            },
            "Check status.",
        );
        routine.guardrails.cooldown = Duration::from_secs(3600);
        db.create_routine(&routine).await.expect("create_routine");
        engine.refresh_event_cache().await;

        // First fire should work.
        let msg = IncomingMessage {
            id: Uuid::new_v4(),
            channel: "test".to_string(),
            user_id: "default".to_string(),
            user_name: None,
            content: "test-cooldown trigger".to_string(),
            thread_id: None,
            received_at: Utc::now(),
            metadata: serde_json::json!({}),
            timezone: None,
            attachments: Vec::new(),
        };
        let fired1 = engine.check_event_triggers(&msg).await;
        assert!(fired1 >= 1, "First fire should work");

        // Give spawn time, then update last_run_at to simulate recent execution.
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Update the routine's last_run_at to now (simulating it just ran).
        db.update_routine_runtime(routine.id, Utc::now(), None, 1, 0, &serde_json::json!({}))
            .await
            .expect("update_routine_runtime");

        // Refresh cache to pick up updated last_run_at.
        engine.refresh_event_cache().await;

        // Second fire should be blocked by cooldown.
        let fired2 = engine.check_event_triggers(&msg).await;
        assert_eq!(fired2, 0, "Second fire should be blocked by cooldown");
    }

    // -----------------------------------------------------------------------
    // Test 5: heartbeat_findings
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn heartbeat_findings() {
        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        // Write a real heartbeat checklist.
        ws.write(
            "HEARTBEAT.md",
            "# Heartbeat Checklist\n\n- [ ] Check if the server is running\n- [ ] Review error logs",
        )
        .await
        .expect("write heartbeat");

        // LLM responds with findings (not HEARTBEAT_OK).
        let trace = LlmTrace::single_turn(
            "test-heartbeat-findings",
            "heartbeat",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "The server has elevated error rates. Review the logs immediately."
                        .to_string(),
                    input_tokens: 100,
                    output_tokens: 20,
                },
                expected_tool_results: vec![],
            }],
        );
        let llm = Arc::new(TraceLlm::from_trace(trace));

        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let hygiene_config = HygieneConfig {
            enabled: false,
            daily_retention_days: 30,
            conversation_retention_days: 7,
            cadence_hours: 24,
            state_dir: _tmp.path().to_path_buf(),
        };

        let runner = HeartbeatRunner::new(HeartbeatConfig::default(), hygiene_config, ws, llm)
            .with_response_channel(tx);

        let result = runner.check_heartbeat().await;
        match result {
            ironclaw::agent::HeartbeatResult::NeedsAttention(msg) => {
                assert!(
                    msg.contains("error"),
                    "Expected 'error' in attention message: {msg}"
                );
            }
            other => panic!("Expected NeedsAttention, got: {other:?}"),
        }

        // No notification since we called check_heartbeat directly (not run).
        let _ = rx.try_recv();
    }

    // -----------------------------------------------------------------------
    // Test 6: heartbeat_empty_skip
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn heartbeat_empty_skip() {
        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        // Write an effectively empty heartbeat (just headers and comments).
        ws.write(
            "HEARTBEAT.md",
            "# Heartbeat Checklist\n\n<!-- No tasks yet -->\n",
        )
        .await
        .expect("write heartbeat");

        // LLM should NOT be called, so provide a trace that would panic if called.
        let trace = LlmTrace::single_turn("test-heartbeat-skip", "skip", vec![]);
        let llm = Arc::new(TraceLlm::from_trace(trace));

        let hygiene_config = HygieneConfig {
            enabled: false,
            daily_retention_days: 30,
            conversation_retention_days: 7,
            cadence_hours: 24,
            state_dir: _tmp.path().to_path_buf(),
        };

        let runner = HeartbeatRunner::new(HeartbeatConfig::default(), hygiene_config, ws, llm);

        let result = runner.check_heartbeat().await;
        assert!(
            matches!(result, ironclaw::agent::HeartbeatResult::Skipped),
            "Expected Skipped for empty checklist, got: {result:?}"
        );
    }
}
