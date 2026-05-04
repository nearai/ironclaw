//! Integration tests for `ironclaw insights` aggregation against a real
//! libSQL database.
//!
//! Covers the four scopes called out in the PR spec:
//!   1. Seed jobs + routine_runs in a temp libSQL, run insights, assert counts.
//!   2. `--json` output matches a snapshot fixture for the seeded data.
//!   3. Zero-data case prints / serializes an empty-state payload.
//!   4. Tool frequency aggregation respects the time window (rows older than
//!      the window must not appear).

#![cfg(feature = "libsql")]

use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;

async fn create_test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let db_path = temp_dir.path().join("insights.db");
    let backend = LibSqlBackend::new_local(&db_path)
        .await
        .expect("LibSqlBackend");
    backend.run_migrations().await.expect("migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, temp_dir)
}

/// Insert a row directly into agent_jobs at a given timestamp.
/// Goes through raw SQL on a fresh libSQL connection because the
/// public `save_job` helper would stamp `created_at` to "now",
/// and we need precise control for the time-window test.
async fn seed_job(
    db_path: &std::path::Path,
    id: &str,
    user_id: &str,
    created_at: chrono::DateTime<chrono::Utc>,
    tokens_used: u64,
) {
    let backend = LibSqlBackend::new_local(db_path).await.expect("seed open");
    let conn = backend.connect().await.expect("seed conn");
    conn.execute(
        "INSERT INTO agent_jobs (id, title, description, status, source, user_id, \
         repair_attempts, max_tokens, total_tokens_used, created_at) \
         VALUES (?1, 'seed', '', 'completed', 'direct', ?2, 0, 0, ?3, ?4)",
        libsql::params![
            id.to_string(),
            user_id.to_string(),
            tokens_used as i64,
            created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ],
    )
    .await
    .expect("seed insert job");
}

async fn seed_action(db_path: &std::path::Path, job_id: &str, seq: i64, tool_name: &str) {
    let backend = LibSqlBackend::new_local(db_path).await.expect("seed open");
    let conn = backend.connect().await.expect("seed conn");
    let action_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO job_actions (id, job_id, sequence_num, tool_name, input, \
         success, created_at) \
         VALUES (?1, ?2, ?3, ?4, '{}', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        libsql::params![action_id, job_id.to_string(), seq, tool_name.to_string()],
    )
    .await
    .expect("seed insert action");
}

/// Insert a routine + a single routine_run at a given timestamp.
/// Used to verify `total_routine_runs` aggregation respects the time window.
async fn seed_routine_run(
    db_path: &std::path::Path,
    user_id: &str,
    created_at: chrono::DateTime<chrono::Utc>,
) {
    let backend = LibSqlBackend::new_local(db_path).await.expect("seed open");
    let conn = backend.connect().await.expect("seed conn");
    let routine_id = uuid::Uuid::new_v4().to_string();
    let run_id = uuid::Uuid::new_v4().to_string();
    let stamp = created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    conn.execute(
        "INSERT INTO routines (id, name, description, user_id, trigger_type, \
         trigger_config, action_type, action_config) VALUES \
         (?1, ?2, '', ?3, 'cron', '{}', 'job', '{}')",
        libsql::params![
            routine_id.clone(),
            format!("test-{}", &routine_id[..8]),
            user_id.to_string(),
        ],
    )
    .await
    .expect("seed insert routine");
    conn.execute(
        "INSERT INTO routine_runs (id, routine_id, trigger_type, status, created_at, started_at) \
         VALUES (?1, ?2, 'cron', 'completed', ?3, ?3)",
        libsql::params![run_id, routine_id, stamp],
    )
    .await
    .expect("seed insert routine_run");
}

#[tokio::test]
async fn aggregate_counts_jobs_and_top_tools_in_window() {
    let (db, tmp) = create_test_db().await;
    let db_path = tmp.path().join("insights.db");

    let now = Utc::now();
    let inside = now - Duration::days(2);
    let job1 = uuid::Uuid::new_v4().to_string();
    let job2 = uuid::Uuid::new_v4().to_string();
    seed_job(&db_path, &job1, "alice", inside, 1_000).await;
    seed_job(&db_path, &job2, "bob", inside, 500).await;

    // alice's job has 3 shell calls + 1 read_file. bob's job has 1 shell.
    seed_action(&db_path, &job1, 1, "shell").await;
    seed_action(&db_path, &job1, 2, "shell").await;
    seed_action(&db_path, &job1, 3, "shell").await;
    seed_action(&db_path, &job1, 4, "read_file").await;
    seed_action(&db_path, &job2, 1, "shell").await;

    let since = now - Duration::days(7);
    let agg = db
        .aggregate_insights(since, now, 10)
        .await
        .expect("aggregate_insights");

    assert_eq!(agg.total_jobs, 2, "should count both seeded jobs");
    assert_eq!(agg.total_tokens_used, 1_500, "should sum total_tokens_used");
    assert_eq!(
        agg.top_tools.first().map(|t| t.tool_name.as_str()),
        Some("shell"),
        "shell should be the top tool"
    );
    assert_eq!(
        agg.top_tools.first().map(|t| t.invocations),
        Some(4),
        "shell should be invoked 4 times across both jobs"
    );
    // top_tools is sorted desc by invocations, and the second entry must
    // exist: read_file was called once.
    assert_eq!(
        agg.top_tools.get(1).map(|t| t.tool_name.as_str()),
        Some("read_file"),
    );
    assert!(
        !agg.daily_activity.is_empty(),
        "daily_activity should have at least one bucket"
    );
}

#[tokio::test]
async fn json_payload_for_seeded_data_matches_fixture_shape() {
    let (db, tmp) = create_test_db().await;
    let db_path = tmp.path().join("insights.db");

    let now = Utc::now();
    let job = uuid::Uuid::new_v4().to_string();
    seed_job(&db_path, &job, "alice", now - Duration::days(1), 42).await;
    seed_action(&db_path, &job, 1, "shell").await;

    let agg = db
        .aggregate_insights(now - Duration::days(7), now, 10)
        .await
        .expect("aggregate_insights");

    // Build the same payload the CLI emits, then strip volatile fields
    // (daily_activity dates depend on wall clock) before comparing.
    let payload = serde_json::json!({
        "version": 1,
        "window_days": 7,
        "total_jobs": agg.total_jobs,
        "total_routine_runs": agg.total_routine_runs,
        "total_tokens_used": agg.total_tokens_used,
        "top_tools": agg.top_tools,
    });
    let expected = serde_json::json!({
        "version": 1,
        "window_days": 7,
        "total_jobs": 1,
        "total_routine_runs": 0,
        "total_tokens_used": 42,
        "top_tools": [
            { "tool_name": "shell", "invocations": 1 }
        ],
    });
    assert_eq!(payload, expected, "JSON shape regression");
}

#[tokio::test]
async fn empty_database_returns_zero_aggregate() {
    let (db, _tmp) = create_test_db().await;
    let now = Utc::now();
    let agg = db
        .aggregate_insights(now - Duration::days(30), now, 10)
        .await
        .expect("aggregate_insights");

    assert_eq!(agg.total_jobs, 0);
    assert_eq!(agg.total_routine_runs, 0);
    assert_eq!(agg.total_tokens_used, 0);
    assert!(agg.top_tools.is_empty());
    assert!(agg.daily_activity.is_empty());
}

#[tokio::test]
async fn time_window_excludes_old_rows() {
    let (db, tmp) = create_test_db().await;
    let db_path = tmp.path().join("insights.db");

    let now = Utc::now();
    let recent_job = uuid::Uuid::new_v4().to_string();
    let old_job = uuid::Uuid::new_v4().to_string();

    seed_job(&db_path, &recent_job, "alice", now - Duration::days(2), 100).await;
    // Older than the window — must NOT be counted.
    seed_job(&db_path, &old_job, "alice", now - Duration::days(60), 9_999).await;

    seed_action(&db_path, &recent_job, 1, "shell").await;
    seed_action(&db_path, &old_job, 1, "shell").await;
    seed_action(&db_path, &old_job, 2, "shell").await;
    seed_action(&db_path, &old_job, 3, "shell").await;

    // Window: last 7 days.
    let agg = db
        .aggregate_insights(now - Duration::days(7), now, 10)
        .await
        .expect("aggregate_insights");

    assert_eq!(
        agg.total_jobs, 1,
        "old_job is outside the window and must not be counted"
    );
    assert_eq!(
        agg.total_tokens_used, 100,
        "tokens from old_job must not leak in"
    );
    // shell from old_job (3 invocations) must not appear; only the 1 from recent_job.
    let shell_count = agg
        .top_tools
        .iter()
        .find(|t| t.tool_name == "shell")
        .map(|t| t.invocations)
        .unwrap_or(0);
    assert_eq!(
        shell_count, 1,
        "tool frequency must respect the time window via the agent_jobs join"
    );
}

#[tokio::test]
async fn aggregate_counts_routine_runs_in_window_only() {
    let (db, tmp) = create_test_db().await;
    let db_path = tmp.path().join("insights.db");

    let now = Utc::now();
    // Two recent runs (must count), one ancient run (must not).
    seed_routine_run(&db_path, "alice", now - Duration::days(1)).await;
    seed_routine_run(&db_path, "alice", now - Duration::days(3)).await;
    seed_routine_run(&db_path, "alice", now - Duration::days(60)).await;

    // Sanity: also seed a job so the empty-table path doesn't accidentally
    // skip the routine_runs query.
    let job = uuid::Uuid::new_v4().to_string();
    seed_job(&db_path, &job, "alice", now - Duration::days(1), 1).await;

    let agg = db
        .aggregate_insights(now - Duration::days(7), now, 10)
        .await
        .expect("aggregate_insights");

    assert_eq!(
        agg.total_routine_runs, 2,
        "expected exactly two recent routine_runs (the 60-day-old one is outside the window)"
    );
}

/// Future-dated rows must NOT leak into the totals. Clock skew, replay
/// tooling, or rows committed in another tick after `until` is captured
/// would otherwise inflate the aggregate.
#[tokio::test]
async fn time_window_excludes_future_rows() {
    let (db, tmp) = create_test_db().await;
    let db_path = tmp.path().join("insights.db");

    let now = Utc::now();
    let recent_job = uuid::Uuid::new_v4().to_string();
    let future_job = uuid::Uuid::new_v4().to_string();

    seed_job(&db_path, &recent_job, "alice", now - Duration::days(1), 100).await;
    // Stamped after `until` — must NOT be counted.
    seed_job(
        &db_path,
        &future_job,
        "alice",
        now + Duration::days(2),
        9_999,
    )
    .await;
    seed_action(&db_path, &recent_job, 1, "shell").await;
    seed_action(&db_path, &future_job, 1, "shell").await;
    seed_action(&db_path, &future_job, 2, "shell").await;
    seed_routine_run(&db_path, "alice", now - Duration::days(1)).await;
    seed_routine_run(&db_path, "alice", now + Duration::days(2)).await;

    // until is pinned at `now` so the future-dated rows are unambiguously
    // outside the window.
    let agg = db
        .aggregate_insights(now - Duration::days(7), now, 10)
        .await
        .expect("aggregate_insights");

    assert_eq!(
        agg.total_jobs, 1,
        "future_job is past the until bound and must not be counted"
    );
    assert_eq!(
        agg.total_tokens_used, 100,
        "tokens from future_job must not leak in"
    );
    assert_eq!(
        agg.total_routine_runs, 1,
        "future-dated routine_run must not be counted"
    );
    let shell_count = agg
        .top_tools
        .iter()
        .find(|t| t.tool_name == "shell")
        .map(|t| t.invocations)
        .unwrap_or(0);
    assert_eq!(
        shell_count, 1,
        "tool frequency must respect the upper bound via the agent_jobs join"
    );
}
