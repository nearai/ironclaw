//! Tests for batch_get_last_run_status (#1469 N+1 fix).
//!
//! Verifies:
//! 1. Empty input returns empty map
//! 2. Returns the most recent run status per routine
//! 3. Routines with no runs are omitted from result
//! 4. Multiple routines with different statuses are correctly returned

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;

    use chrono::{Duration, Utc};
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

    fn make_routine(id: Uuid) -> Routine {
        Routine {
            id,
            name: format!("test-routine-{}", id),
            description: "Test routine".to_string(),
            user_id: "default".to_string(),
            enabled: true,
            trigger: Trigger::Manual,
            action: RoutineAction::FullJob {
                title: "Test job".to_string(),
                description: "Test description".to_string(),
                max_iterations: 5,
            },
            guardrails: RoutineGuardrails {
                cooldown: std::time::Duration::from_secs(0),
                max_concurrent: 1,
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
        }
    }

    fn make_run(routine_id: Uuid, status: RunStatus, started_at: chrono::DateTime<chrono::Utc>) -> RoutineRun {
        RoutineRun {
            id: Uuid::new_v4(),
            routine_id,
            trigger_type: "manual".to_string(),
            trigger_detail: None,
            started_at,
            completed_at: if status == RunStatus::Running { None } else { Some(Utc::now()) },
            status,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_batch_get_last_run_status_empty_input() {
        let (db, _tmp) = create_test_db().await;
        let result = db.batch_get_last_run_status(&[]).await.expect("batch query");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_batch_get_last_run_status_returns_latest() {
        let (db, _tmp) = create_test_db().await;

        let routine_id = Uuid::new_v4();
        db.create_routine(&make_routine(routine_id)).await.expect("create routine");

        // Create an older run with Ok status
        let older_run = make_run(routine_id, RunStatus::Ok, Utc::now() - Duration::hours(2));
        db.create_routine_run(&older_run).await.expect("create older run");
        db.complete_routine_run(older_run.id, RunStatus::Ok, None, None).await.expect("complete older run");

        // Create a newer run with Attention status
        let newer_run = make_run(routine_id, RunStatus::Attention, Utc::now() - Duration::hours(1));
        db.create_routine_run(&newer_run).await.expect("create newer run");
        db.complete_routine_run(newer_run.id, RunStatus::Attention, None, None).await.expect("complete newer run");

        let result = db.batch_get_last_run_status(&[routine_id]).await.expect("batch query");
        assert_eq!(result.get(&routine_id), Some(&RunStatus::Attention));
    }

    #[tokio::test]
    async fn test_batch_get_last_run_status_omits_routines_without_runs() {
        let (db, _tmp) = create_test_db().await;

        let with_runs = Uuid::new_v4();
        let without_runs = Uuid::new_v4();
        db.create_routine(&make_routine(with_runs)).await.expect("create routine");
        db.create_routine(&make_routine(without_runs)).await.expect("create routine");

        let run = make_run(with_runs, RunStatus::Ok, Utc::now());
        db.create_routine_run(&run).await.expect("create run");
        db.complete_routine_run(run.id, RunStatus::Ok, None, None).await.expect("complete run");

        let result = db.batch_get_last_run_status(&[with_runs, without_runs]).await.expect("batch query");
        assert_eq!(result.get(&with_runs), Some(&RunStatus::Ok));
        assert_eq!(result.get(&without_runs), None);
    }

    #[tokio::test]
    async fn test_batch_get_last_run_status_multiple_routines() {
        let (db, _tmp) = create_test_db().await;

        let r1 = Uuid::new_v4();
        let r2 = Uuid::new_v4();
        db.create_routine(&make_routine(r1)).await.expect("create r1");
        db.create_routine(&make_routine(r2)).await.expect("create r2");

        let run1 = make_run(r1, RunStatus::Running, Utc::now());
        db.create_routine_run(&run1).await.expect("create run1");

        let run2 = make_run(r2, RunStatus::Failed, Utc::now());
        db.create_routine_run(&run2).await.expect("create run2");
        db.complete_routine_run(run2.id, RunStatus::Failed, None, None).await.expect("complete run2");

        let result = db.batch_get_last_run_status(&[r1, r2]).await.expect("batch query");
        assert_eq!(result.get(&r1), Some(&RunStatus::Running));
        assert_eq!(result.get(&r2), Some(&RunStatus::Failed));
    }
}
