//! Tests for batch loading routine concurrent counts (N+1 query fix).
//!
//! Verifies:
//! 1. Batch query returns correct counts for multiple routines
//! 2. Concurrent limit enforcement uses batch counts correctly

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use uuid::Uuid;

    use ironclaw::agent::routine::{
        Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
    };
    use ironclaw::db::Database;

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

    // -----------------------------------------------------------------------
    // Test 1: Batch query returns correct counts for multiple routines
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn batch_query_empty_list() {
        let (db, _tmp) = create_test_db().await;
        let counts = db
            .count_running_routine_runs_batch(&[])
            .await
            .expect("batch query should not fail");
        assert!(counts.is_empty(), "Empty input should return empty map");
    }

    #[tokio::test]
    async fn batch_query_single_routine() {
        let (db, _tmp) = create_test_db().await;
        let routine_id = Uuid::new_v4();

        // Create routine
        let routine = Routine {
            id: routine_id,
            name: "test-routine".to_string(),
            description: "Test".to_string(),
            user_id: "default".to_string(),
            workspace_id: None,
            enabled: true,
            trigger: Trigger::Cron {
                schedule: "* * * * *".to_string(),
                timezone: None,
            },
            action: RoutineAction::Lightweight {
                prompt: "test".to_string(),
                context_paths: vec![],
                max_tokens: 1000,
                use_tools: false,
                max_tool_rounds: 3,
            },
            guardrails: RoutineGuardrails {
                cooldown: std::time::Duration::from_secs(0),
                max_concurrent: 5,
                dedup_window: None,
            },
            notify: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.create_routine(&routine).await.expect("create routine");

        // Create 3 running runs
        for _ in 0..3 {
            let run = RoutineRun {
                id: Uuid::new_v4(),
                routine_id,
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
            db.create_routine_run(&run).await.expect("create run");
        }

        // Batch query for single routine
        let counts = db
            .count_running_routine_runs_batch(&[routine_id])
            .await
            .expect("batch query should work");

        assert_eq!(counts.len(), 1, "Should return 1 routine");
        assert_eq!(counts[&routine_id], 3, "Should count 3 running runs");
    }

    #[tokio::test]
    async fn batch_query_multiple_routines_different_counts() {
        let (db, _tmp) = create_test_db().await;

        let r1 = Uuid::new_v4();
        let r2 = Uuid::new_v4();
        let r3 = Uuid::new_v4();

        // Create 3 routines
        for routine_id in [r1, r2, r3] {
            let routine = Routine {
                id: routine_id,
                name: format!("routine-{}", routine_id),
                description: "Test".to_string(),
                user_id: "default".to_string(),
                workspace_id: None,
                enabled: true,
                trigger: Trigger::Cron {
                    schedule: "* * * * *".to_string(),
                    timezone: None,
                },
                action: RoutineAction::Lightweight {
                    prompt: "test".to_string(),
                    context_paths: vec![],
                    max_tokens: 1000,
                    use_tools: false,
                    max_tool_rounds: 3,
                },
                guardrails: RoutineGuardrails {
                    cooldown: std::time::Duration::from_secs(0),
                    max_concurrent: 5,
                    dedup_window: None,
                },
                notify: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            db.create_routine(&routine).await.expect("create routine");
        }

        // r1: 2 running
        for _ in 0..2 {
            let run = RoutineRun {
                id: Uuid::new_v4(),
                routine_id: r1,
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
            db.create_routine_run(&run).await.expect("create run");
        }

        // r2: 1 running
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: r2,
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
        db.create_routine_run(&run).await.expect("create run");

        // r3: 0 running (but has 1 Ok result)
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: r3,
            trigger_type: "cron".to_string(),
            trigger_detail: None,
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            status: RunStatus::Ok,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };
        db.create_routine_run(&run).await.expect("create run");

        // Single batch query for all 3
        let counts = db
            .count_running_routine_runs_batch(&[r1, r2, r3])
            .await
            .expect("batch query should work");

        assert_eq!(counts.len(), 3, "Should return 3 routines");
        assert_eq!(counts[&r1], 2, "r1 should have 2 running");
        assert_eq!(counts[&r2], 1, "r2 should have 1 running");
        assert_eq!(
            counts[&r3], 0,
            "r3 should have 0 running (Ok status is not running)"
        );
    }

    #[tokio::test]
    async fn batch_query_missing_routines_default_to_zero() {
        let (db, _tmp) = create_test_db().await;

        let r1 = Uuid::new_v4();
        let r2 = Uuid::new_v4();
        let r3 = Uuid::new_v4(); // This one won't exist

        // Only create r1
        let routine = Routine {
            id: r1,
            name: "routine-1".to_string(),
            description: "Test".to_string(),
            user_id: "default".to_string(),
            workspace_id: None,
            enabled: true,
            trigger: Trigger::Cron {
                schedule: "* * * * *".to_string(),
                timezone: None,
            },
            action: RoutineAction::Lightweight {
                prompt: "test".to_string(),
                context_paths: vec![],
                max_tokens: 1000,
                use_tools: false,
                max_tool_rounds: 3,
            },
            guardrails: RoutineGuardrails {
                cooldown: std::time::Duration::from_secs(0),
                max_concurrent: 5,
                dedup_window: None,
            },
            notify: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.create_routine(&routine).await.expect("create routine");

        // r1 has 1 running
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: r1,
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
        db.create_routine_run(&run).await.expect("create run");

        // Query for r1, r2 (doesn't exist), r3 (doesn't exist)
        let counts = db
            .count_running_routine_runs_batch(&[r1, r2, r3])
            .await
            .expect("batch query should work");

        assert_eq!(counts.len(), 3, "Should have all 3 routine IDs");
        assert_eq!(counts[&r1], 1, "r1 should have 1 running");
        assert_eq!(counts[&r2], 0, "r2 should default to 0");
        assert_eq!(counts[&r3], 0, "r3 should default to 0");
    }

    #[tokio::test]
    async fn batch_query_only_counts_running_status() {
        let (db, _tmp) = create_test_db().await;
        let routine_id = Uuid::new_v4();

        // Create routine
        let routine = Routine {
            id: routine_id,
            name: "test-routine".to_string(),
            description: "Test".to_string(),
            user_id: "default".to_string(),
            workspace_id: None,
            enabled: true,
            trigger: Trigger::Cron {
                schedule: "* * * * *".to_string(),
                timezone: None,
            },
            action: RoutineAction::Lightweight {
                prompt: "test".to_string(),
                context_paths: vec![],
                max_tokens: 1000,
                use_tools: false,
                max_tool_rounds: 3,
            },
            guardrails: RoutineGuardrails {
                cooldown: std::time::Duration::from_secs(0),
                max_concurrent: 5,
                dedup_window: None,
            },
            notify: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.create_routine(&routine).await.expect("create routine");

        // Create 5 runs with mixed statuses
        let statuses = [
            RunStatus::Running,
            RunStatus::Running,
            RunStatus::Ok,
            RunStatus::Failed,
            RunStatus::Attention,
        ];

        for status in statuses.iter() {
            let run = RoutineRun {
                id: Uuid::new_v4(),
                routine_id,
                trigger_type: "cron".to_string(),
                trigger_detail: None,
                started_at: Utc::now(),
                completed_at: Some(Utc::now()),
                status: *status,
                result_summary: None,
                tokens_used: None,
                job_id: None,
                created_at: Utc::now(),
            };
            db.create_routine_run(&run).await.expect("create run");
        }

        // Batch query should only count Running status
        let counts = db
            .count_running_routine_runs_batch(&[routine_id])
            .await
            .expect("batch query should work");

        assert_eq!(
            counts[&routine_id], 2,
            "Should only count 2 Running status runs"
        );
    }

    // -----------------------------------------------------------------------
    // Test 2: Concurrent limit enforcement uses batch counts
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn concurrent_limit_enforcement_with_batch_counts() {
        let (db, _tmp) = create_test_db().await;

        let r1 = Uuid::new_v4();
        let r2 = Uuid::new_v4();

        // Create 2 routines with max_concurrent=1 (r1) and max_concurrent=2 (r2)
        for (routine_id, max_concurrent) in [(r1, 1), (r2, 2)] {
            let routine = Routine {
                id: routine_id,
                name: format!("routine-{}", routine_id),
                description: "Test".to_string(),
                user_id: "default".to_string(),
                workspace_id: None,
                enabled: true,
                trigger: Trigger::Cron {
                    schedule: "* * * * *".to_string(),
                    timezone: None,
                },
                action: RoutineAction::Lightweight {
                    prompt: "test".to_string(),
                    context_paths: vec![],
                    max_tokens: 1000,
                    use_tools: false,
                    max_tool_rounds: 3,
                },
                guardrails: RoutineGuardrails {
                    cooldown: std::time::Duration::from_secs(0),
                    max_concurrent,
                    dedup_window: None,
                },
                notify: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            db.create_routine(&routine).await.expect("create routine");
        }

        // r1: create 1 running run (will hit max_concurrent=1)
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: r1,
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
        db.create_routine_run(&run).await.expect("create run");

        // r2: create 2 running runs (will hit max_concurrent=2)
        for _ in 0..2 {
            let run = RoutineRun {
                id: Uuid::new_v4(),
                routine_id: r2,
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
            db.create_routine_run(&run).await.expect("create run");
        }

        // Batch query should return correct counts
        let counts = db
            .count_running_routine_runs_batch(&[r1, r2])
            .await
            .expect("batch query should work");

        // Verify counts match the limits
        assert_eq!(
            counts[&r1], 1,
            "r1 should have 1 running (at max_concurrent=1)"
        );
        assert_eq!(
            counts[&r2], 2,
            "r2 should have 2 running (at max_concurrent=2)"
        );

        // Now verify the limit enforcement logic
        let r1_routine = db
            .get_routine(r1)
            .await
            .expect("get routine")
            .expect("routine exists");
        let r2_routine = db
            .get_routine(r2)
            .await
            .expect("get routine")
            .expect("routine exists");

        let r1_at_limit = counts[&r1] >= r1_routine.guardrails.max_concurrent as i64;
        let r2_at_limit = counts[&r2] >= r2_routine.guardrails.max_concurrent as i64;

        assert!(r1_at_limit, "r1 should be detected as at limit");
        assert!(r2_at_limit, "r2 should be detected as at limit");

        // If we add one more run to r2, it should exceed limit
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: r2,
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
        db.create_routine_run(&run).await.expect("create run");

        // Re-query to get updated counts
        let counts = db
            .count_running_routine_runs_batch(&[r1, r2])
            .await
            .expect("batch query should work");

        let r2_exceeded_limit = counts[&r2] > r2_routine.guardrails.max_concurrent as i64;
        assert!(r2_exceeded_limit, "r2 should have exceeded its limit");
    }
}
