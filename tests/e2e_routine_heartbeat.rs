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

    fn make_message(
        channel: &str,
        user_id: &str,
        owner_id: &str,
        sender_id: &str,
        content: &str,
    ) -> IncomingMessage {
        IncomingMessage::new(channel, user_id, content)
            .with_owner_id(owner_id)
            .with_sender_id(sender_id)
            .with_metadata(serde_json::json!({}))
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
                use_tools: false,
                max_tool_rounds: 3,
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
        let matching_msg = make_message(
            "test",
            "default",
            "default",
            "default",
            "deploy to production now",
        );
        let fired = engine
            .check_event_triggers(
                &matching_msg.user_id,
                &matching_msg.channel,
                &matching_msg.content,
            )
            .await;
        assert!(
            fired >= 1,
            "Expected >= 1 routine fired on match, got {fired}"
        );

        // Give spawn time.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Negative match: message that doesn't match.
        let non_matching_msg = make_message(
            "test",
            "default",
            "default",
            "default",
            "check the staging environment",
        );
        let fired_neg = engine
            .check_event_triggers(
                &non_matching_msg.user_id,
                &non_matching_msg.channel,
                &non_matching_msg.content,
            )
            .await;
        assert_eq!(fired_neg, 0, "Expected 0 routines fired on non-match");
    }

    #[tokio::test]
    async fn event_trigger_respects_message_user_scope() {
        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        let trace = LlmTrace::single_turn(
            "test-event-user-scope",
            "deploy",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "Owner event handled".to_string(),
                    input_tokens: 50,
                    output_tokens: 8,
                },
                expected_tool_results: vec![],
            }],
        );
        let llm = Arc::new(TraceLlm::from_trace(trace));
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::channel(16);

        let tools = Arc::new(ToolRegistry::new());
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }));

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

        let routine = make_routine(
            "owner-deploy-watcher",
            Trigger::Event {
                channel: None,
                pattern: "deploy.*production".to_string(),
            },
            "Report on deployment.",
        );
        db.create_routine(&routine).await.expect("create_routine");
        engine.refresh_event_cache().await;

        let guest_msg = make_message(
            "telegram",
            "guest",
            "default",
            "guest-sender",
            "deploy to production now",
        );
        let guest_fired = engine
            .check_event_triggers(&guest_msg.user_id, &guest_msg.channel, &guest_msg.content)
            .await;
        assert_eq!(
            guest_fired, 0,
            "Guest scope must not fire owner event routines"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;

        let guest_runs = db
            .list_routine_runs(routine.id, 10)
            .await
            .expect("list_routine_runs after guest message");
        assert!(
            guest_runs.is_empty(),
            "Guest message should not create routine runs"
        );

        let owner_msg = make_message(
            "telegram",
            "default",
            "default",
            "owner-sender",
            "deploy to production now",
        );
        let owner_fired = engine
            .check_event_triggers(&owner_msg.user_id, &owner_msg.channel, &owner_msg.content)
            .await;
        assert!(
            owner_fired >= 1,
            "Owner scope should fire matching owner event routine"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;

        let owner_runs = db
            .list_routine_runs(routine.id, 10)
            .await
            .expect("list_routine_runs after owner message");
        assert_eq!(
            owner_runs.len(),
            1,
            "Owner message should create exactly one run"
        );
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
        let msg = make_message(
            "test",
            "default",
            "default",
            "default",
            "test-cooldown trigger",
        );
        let fired1 = engine
            .check_event_triggers(&msg.user_id, &msg.channel, &msg.content)
            .await;
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
        let fired2 = engine
            .check_event_triggers(&msg.user_id, &msg.channel, &msg.content)
            .await;
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

    /// Helper to set up a test environment for routine engine mutation tests.
    /// Returns the engine, database, and temp directory.
    async fn setup_routine_mutation_test()
    -> (Arc<RoutineEngine>, Arc<dyn Database>, tempfile::TempDir) {
        let (db, dir) = create_test_db().await;
        let ws = create_workspace(&db);
        let (notify_tx, _rx) = tokio::sync::mpsc::channel(16);
        let tools = Arc::new(ToolRegistry::new());

        let safety_config = SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        };
        let safety = Arc::new(SafetyLayer::new(&safety_config));

        let trace = LlmTrace::single_turn(
            "test-routine-mutation",
            "test",
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

        let engine = Arc::new(RoutineEngine::new(
            RoutineConfig::default(),
            Arc::clone(&db),
            llm,
            ws,
            notify_tx,
            None,
            tools,
            safety,
        ));

        (engine, db, dir)
    }

    /// Regression test for issue #1076: disabling an event routine via a DB mutation
    /// followed by refresh_event_cache() (the path now taken by the web toggle handler)
    /// must immediately stop the routine from firing.
    #[tokio::test]
    async fn toggle_disabling_event_routine_removes_from_cache() {
        let (engine, db, _dir) = setup_routine_mutation_test().await;

        // Create and cache an event routine.
        let mut routine = make_routine(
            "disable-me",
            Trigger::Event {
                pattern: "DISABLE_ME".to_string(),
                channel: None,
            },
            "Handle DISABLE_ME event",
        );
        db.create_routine(&routine).await.expect("create_routine");
        engine.refresh_event_cache().await;

        let msg = IncomingMessage::new("test", "default", "DISABLE_ME");
        let fired_before = engine
            .check_event_triggers(&msg.user_id, &msg.channel, &msg.content)
            .await;
        assert!(fired_before >= 1, "Expected routine to fire before disable");

        // Simulate what routines_toggle_handler now does: update DB, then refresh.
        routine.enabled = false;
        routine.updated_at = Utc::now();
        db.update_routine(&routine).await.expect("update_routine");
        engine.refresh_event_cache().await;

        let fired_after = engine
            .check_event_triggers(&msg.user_id, &msg.channel, &msg.content)
            .await;
        assert_eq!(
            fired_after, 0,
            "Disabled routine must not fire after cache refresh"
        );
    }

    /// Regression test for issue #1076: deleting an event routine via a DB mutation
    /// followed by refresh_event_cache() must immediately stop the routine from firing.
    #[tokio::test]
    async fn delete_event_routine_removes_from_cache() {
        let (engine, db, _dir) = setup_routine_mutation_test().await;

        let routine = make_routine(
            "delete-me",
            Trigger::Event {
                pattern: "DELETE_ME".to_string(),
                channel: None,
            },
            "Handle DELETE_ME event",
        );
        db.create_routine(&routine).await.expect("create_routine");
        engine.refresh_event_cache().await;

        let msg = IncomingMessage::new("test", "default", "DELETE_ME");
        assert!(
            engine
                .check_event_triggers(&msg.user_id, &msg.channel, &msg.content)
                .await
                >= 1,
            "Expected routine to fire before delete"
        );

        // Simulate what routines_delete_handler now does: delete from DB, then refresh.
        db.delete_routine(routine.id).await.expect("delete_routine");
        engine.refresh_event_cache().await;

        assert_eq!(
            engine
                .check_event_triggers(&msg.user_id, &msg.channel, &msg.content)
                .await,
            0,
            "Deleted routine must not fire after cache refresh"
        );
    }

    // -----------------------------------------------------------------------
    // Test: full_job per-routine concurrency blocks second fire (issue #1318)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn full_job_max_concurrent_blocks_second_fire_while_first_active() {
        use ironclaw::agent::routine::{
            NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
        };
        use ironclaw::error::RoutineError;

        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        // Stub LLM — fire_manual will be rejected before any LLM call
        let trace = LlmTrace::single_turn(
            "stub",
            "stub",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "ROUTINE_OK".to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                },
                expected_tool_results: vec![],
            }],
        );
        let llm = Arc::new(TraceLlm::from_trace(trace));
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::channel(4);
        let tools = Arc::new(ToolRegistry::new());
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
        }));

        let engine = Arc::new(RoutineEngine::new(
            RoutineConfig::default(),
            db.clone(),
            llm,
            ws,
            notify_tx,
            None, // no scheduler — rejected before dispatch
            tools,
            safety,
        ));

        // Create a full_job routine with max_concurrent = 1
        let routine = Routine {
            id: Uuid::new_v4(),
            name: "concurrent-guard".to_string(),
            description: "test max_concurrent for full_job".to_string(),
            user_id: "default".to_string(),
            enabled: true,
            trigger: Trigger::Manual,
            action: RoutineAction::FullJob {
                title: "t".to_string(),
                description: "d".to_string(),
                max_iterations: 3,
                tool_permissions: vec![],
            },
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(0),
                max_concurrent: 1,
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
        };
        db.create_routine(&routine).await.expect("create_routine");

        // Simulate first full_job run still active: the fix keeps the
        // routine_run in Running state while the linked job executes.
        let active_run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: routine.id,
            trigger_type: "cron".to_string(),
            trigger_detail: None,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };
        db.create_routine_run(&active_run)
            .await
            .expect("create_routine_run");

        // Attempt to fire the same routine again — must be rejected
        let result = engine.fire_manual(routine.id, None).await;
        assert!(
            matches!(result, Err(RoutineError::MaxConcurrent { .. })),
            "second fire while first full_job active must be rejected by max_concurrent=1, got: {:?}",
            result
        );
    }

    // -----------------------------------------------------------------------
    // Test: global running_count tracks live full_job runs (issue #1318)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn global_concurrency_counts_live_full_job_runs() {
        use std::sync::atomic::Ordering;

        let (db, _tmp) = create_test_db().await;
        let ws = create_workspace(&db);

        let trace = LlmTrace::single_turn(
            "test-global-limit",
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
        let tools = Arc::new(ToolRegistry::new());
        let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: true,
        }));

        // Configure global limit of 1
        let config = RoutineConfig {
            max_concurrent_routines: 1,
            ..RoutineConfig::default()
        };

        let engine = Arc::new(RoutineEngine::new(
            config,
            db.clone(),
            llm,
            ws,
            notify_tx,
            None,
            tools,
            safety,
        ));

        // Insert a due cron routine
        let mut routine = make_routine(
            "global-limit-test",
            Trigger::Cron {
                schedule: "* * * * *".to_string(),
                timezone: None,
            },
            "Check status.",
        );
        routine.next_fire_at = Some(Utc::now() - chrono::Duration::minutes(1));
        db.create_routine(&routine).await.expect("create_routine");

        // Simulate one full_job from another routine holding the global slot.
        // With the fix, running_count stays elevated for the full job duration.
        engine
            .running_count_for_test()
            .fetch_add(1, Ordering::Relaxed);

        // check_cron_triggers should see global limit hit and skip
        engine.check_cron_triggers().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let runs = db
            .list_routine_runs(routine.id, 10)
            .await
            .expect("list_routine_runs");
        assert!(
            runs.is_empty(),
            "cron routine must not fire when global limit is reached by live full_job"
        );

        // Release the global slot
        engine
            .running_count_for_test()
            .fetch_sub(1, Ordering::Relaxed);

        // Now the routine should fire
        engine.check_cron_triggers().await;
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Because the first check skipped it, next_fire_at is unchanged —
        // the second check should see it as still due and fire it.
        let runs_after = db
            .list_routine_runs(routine.id, 10)
            .await
            .expect("list_routine_runs");
        assert!(
            !runs_after.is_empty(),
            "cron routine should fire after global slot is released"
        );
    }
}
