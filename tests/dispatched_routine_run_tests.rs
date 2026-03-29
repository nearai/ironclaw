//! Integration tests for dispatched routine run tracking (#1317).
//!
//! Verifies:
//! 1. list_dispatched_routine_runs returns only running runs with linked jobs
//! 2. Completed jobs cause linked routine runs to be finalized as Ok
//! 3. Failed jobs cause linked routine runs to be finalized as Failed
//! 4. Active (InProgress) jobs are not finalized
//! 5. Orphaned runs (job_id set but no job record) are handled

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use uuid::Uuid;

    use ironclaw::agent::routine::{
        Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
    };
    use ironclaw::context::{JobContext, JobState};
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

    fn make_run(routine_id: Uuid, job_id: Option<Uuid>) -> RoutineRun {
        RoutineRun {
            id: Uuid::new_v4(),
            routine_id,
            trigger_type: "manual".to_string(),
            trigger_detail: None,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id,
            created_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: list_dispatched_routine_runs returns only running runs with jobs
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_dispatched_returns_only_running_with_job_id() {
        let (db, _tmp) = create_test_db().await;
        let routine_id = Uuid::new_v4();
        let routine = make_routine(routine_id);
        db.create_routine(&routine).await.expect("create routine");

        // Create jobs first (FK constraint requires job records to exist)
        let job1 = JobContext::new("Job 1", "Dispatched job");
        db.save_job(&job1).await.expect("save job1");
        let job2 = JobContext::new("Job 2", "Completed job");
        db.save_job(&job2).await.expect("save job2");

        // Create a running run WITH job_id (dispatched full_job)
        let dispatched_run = make_run(routine_id, Some(job1.job_id));
        db.create_routine_run(&dispatched_run)
            .await
            .expect("create dispatched run");

        // Create a running run WITHOUT job_id (lightweight in-progress)
        let lightweight_run = make_run(routine_id, None);
        db.create_routine_run(&lightweight_run)
            .await
            .expect("create lightweight run");

        // Create a completed run WITH job_id (already finalized)
        let mut completed_run = make_run(routine_id, Some(job2.job_id));
        completed_run.status = RunStatus::Ok;
        completed_run.completed_at = Some(Utc::now());
        db.create_routine_run(&completed_run)
            .await
            .expect("create completed run");

        let dispatched = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched");

        assert_eq!(dispatched.len(), 1, "Should return only the dispatched run");
        assert_eq!(dispatched[0].id, dispatched_run.id);
        assert_eq!(dispatched[0].job_id, Some(job1.job_id));
        assert_eq!(dispatched[0].status, RunStatus::Running);
    }

    // -----------------------------------------------------------------------
    // Test 2: Completed job linked to run can be detected
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn dispatched_run_with_completed_job_can_be_finalized() {
        let (db, _tmp) = create_test_db().await;
        let routine_id = Uuid::new_v4();
        let routine = make_routine(routine_id);
        db.create_routine(&routine).await.expect("create routine");

        // Create and save a job in Completed state
        let mut job = JobContext::new("Test job", "Test description");
        job.state = JobState::Completed;
        db.save_job(&job).await.expect("save job");

        // Create a dispatched run linked to that job
        let run = make_run(routine_id, Some(job.job_id));
        db.create_routine_run(&run).await.expect("create run");

        // Verify the run is listed as dispatched
        let dispatched = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched");
        assert_eq!(dispatched.len(), 1);

        // Verify we can fetch the linked job and see it's completed
        let fetched_job = db
            .get_job(job.job_id)
            .await
            .expect("get job")
            .expect("job should exist");
        assert_eq!(fetched_job.state, JobState::Completed);

        // Simulate sync: complete the run
        db.complete_routine_run(run.id, RunStatus::Ok, Some("Job completed"), None)
            .await
            .expect("complete run");

        // Run should no longer appear in dispatched list
        let dispatched_after = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched after");
        assert!(
            dispatched_after.is_empty(),
            "Finalized run should not appear in dispatched list"
        );
    }

    // -----------------------------------------------------------------------
    // Test 3: Failed job causes run to be finalized as Failed
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn dispatched_run_with_failed_job() {
        let (db, _tmp) = create_test_db().await;
        let routine_id = Uuid::new_v4();
        let routine = make_routine(routine_id);
        db.create_routine(&routine).await.expect("create routine");

        let mut job = JobContext::new("Failing job", "Will fail");
        job.state = JobState::Failed;
        db.save_job(&job).await.expect("save job");

        let run = make_run(routine_id, Some(job.job_id));
        db.create_routine_run(&run).await.expect("create run");

        // Verify job is failed
        let fetched_job = db
            .get_job(job.job_id)
            .await
            .expect("get job")
            .expect("job should exist");
        assert_eq!(fetched_job.state, JobState::Failed);

        // Simulate sync: complete the run as failed
        db.complete_routine_run(run.id, RunStatus::Failed, Some("Job failed"), None)
            .await
            .expect("complete run as failed");

        let dispatched = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched");
        assert!(dispatched.is_empty(), "Failed run should be finalized");
    }

    // -----------------------------------------------------------------------
    // Test 4: Active (InProgress) job leaves run as running
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn dispatched_run_with_active_job_stays_running() {
        let (db, _tmp) = create_test_db().await;
        let routine_id = Uuid::new_v4();
        let routine = make_routine(routine_id);
        db.create_routine(&routine).await.expect("create routine");

        let mut job = JobContext::new("Active job", "Still running");
        job.state = JobState::InProgress;
        db.save_job(&job).await.expect("save job");

        let run = make_run(routine_id, Some(job.job_id));
        db.create_routine_run(&run).await.expect("create run");

        // Verify job is still active
        let fetched_job = db
            .get_job(job.job_id)
            .await
            .expect("get job")
            .expect("job should exist");
        assert!(!fetched_job.state.is_terminal());

        // Run should still be in dispatched list (not finalized)
        let dispatched = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched");
        assert_eq!(
            dispatched.len(),
            1,
            "Run with active job should remain dispatched"
        );
        assert_eq!(dispatched[0].status, RunStatus::Running);
    }

    // -----------------------------------------------------------------------
    // Test 5: Orphaned run (job_id set but job record missing)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn dispatched_run_orphan_detection() {
        let (db, _tmp) = create_test_db().await;
        let routine_id = Uuid::new_v4();
        let routine = make_routine(routine_id);
        db.create_routine(&routine).await.expect("create routine");

        // Create a real job so the FK constraint is satisfied
        let job = JobContext::new("Will be orphaned", "Test orphan detection");
        db.save_job(&job).await.expect("save job");

        let run = make_run(routine_id, Some(job.job_id));
        db.create_routine_run(&run).await.expect("create run");

        // The run appears in dispatched list
        let dispatched = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched");
        assert_eq!(dispatched.len(), 1);

        // Verify orphan detection: a random UUID returns None from get_job
        let nonexistent_id = Uuid::new_v4();
        let missing = db
            .get_job(nonexistent_id)
            .await
            .expect("get_job should not error");
        assert!(
            missing.is_none(),
            "get_job for nonexistent ID should return None"
        );

        // Simulate sync handling of an orphaned run: mark as failed
        db.complete_routine_run(
            run.id,
            RunStatus::Failed,
            Some(&format!("Linked job {} not found (orphaned)", job.job_id)),
            None,
        )
        .await
        .expect("complete orphaned run");

        let dispatched_after = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched after");
        assert!(
            dispatched_after.is_empty(),
            "Finalized run should not appear in dispatched list"
        );
    }

    // -----------------------------------------------------------------------
    // Test 6: link_routine_run_to_job then list shows linked run
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn link_and_list_dispatched_run() {
        let (db, _tmp) = create_test_db().await;
        let routine_id = Uuid::new_v4();
        let routine = make_routine(routine_id);
        db.create_routine(&routine).await.expect("create routine");

        // Create job record (FK constraint)
        let job = JobContext::new("Linked job", "Test linking");
        db.save_job(&job).await.expect("save job");

        // Create a running run without job_id initially
        let run = make_run(routine_id, None);
        db.create_routine_run(&run).await.expect("create run");

        // Should not appear in dispatched list yet
        let dispatched = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched");
        assert!(
            dispatched.is_empty(),
            "Run without job_id should not be dispatched"
        );

        // Link the run to the job
        db.link_routine_run_to_job(run.id, job.job_id)
            .await
            .expect("link run to job");

        // Now it should appear
        let dispatched_after = db
            .list_dispatched_routine_runs()
            .await
            .expect("list dispatched after link");
        assert_eq!(
            dispatched_after.len(),
            1,
            "Linked run should appear in dispatched list"
        );
        assert_eq!(dispatched_after[0].job_id, Some(job.job_id));
    }
}
