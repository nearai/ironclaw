//! Integration tests for the gateway-ops trace replay harness (#643).
//!
//! Exercises `TraceRunner` end-to-end against a real libSQL test database,
//! ensuring the `Tool -> ActionRecord -> save_action` pipeline matches the
//! declared `TraceExpectation`s in each fixture.

#![cfg(feature = "libsql")]

mod support {
    pub mod trace_runner;
}

use std::path::PathBuf;
use std::sync::Arc;

use ironclaw::db::Database;
use ironclaw::db::libsql::LibSqlBackend;
use ironclaw::tools::ToolRegistry;

use support::trace_runner::{Trace, TraceResult, TraceRunner};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gateway_traces")
}

fn load_trace(name: &str) -> Trace {
    let path = fixture_dir().join(format!("{name}.json"));
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("parse fixture {}: {e}", path.display()))
}

/// Build a minimal libSQL-backed fixture: fresh DB + migrations +
/// built-in tool registry. Returns the pieces the runner needs.
///
/// The `_temp_dir` guard must stay alive for the whole test so the
/// database file isn't deleted mid-run.
async fn fixture() -> (Arc<dyn Database>, Arc<ToolRegistry>, tempfile::TempDir) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let db_path = temp_dir.path().join("trace_runner.db");
    let backend = LibSqlBackend::new_local(&db_path)
        .await
        .expect("LibSqlBackend");
    backend.run_migrations().await.expect("migrations");
    let db: Arc<dyn Database> = Arc::new(backend);

    let tools = Arc::new(ToolRegistry::new());
    tools.register_builtin_tools();

    (db, tools, temp_dir)
}

async fn run_trace(name: &str) -> (TraceResult, Arc<dyn Database>, tempfile::TempDir) {
    let (db, tools, temp_dir) = fixture().await;
    let runner = TraceRunner::new(Arc::clone(&tools), Arc::clone(&db), "trace-user");
    let trace = load_trace(name);
    let result = runner.replay(&trace).await.expect("replay should succeed");
    (result, db, temp_dir)
}

#[tokio::test]
async fn echo_roundtrip_records_one_success() {
    let (result, _db, _guard) = run_trace("echo_roundtrip").await;
    assert!(
        result.failures.is_empty(),
        "unexpected failures: {:#?}",
        result.failures
    );
    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert!(record.success);
    assert_eq!(record.tool_name, "echo");
    assert_eq!(record.sequence, 0);
}

#[tokio::test]
async fn idempotency_trace_produces_equal_outputs() {
    let (result, _db, _guard) = run_trace("idempotency").await;
    assert!(
        result.failures.is_empty(),
        "unexpected failures: {:#?}",
        result.failures
    );
    assert_eq!(result.records.len(), 2);
    assert!(result.records.iter().all(|r| r.success));
    assert_eq!(
        result.records[0].output_raw, result.records[1].output_raw,
        "idempotent echo should produce byte-identical raw output across invocations"
    );
}

#[tokio::test]
async fn unknown_tool_is_recorded_as_failure() {
    let (result, _db, _guard) = run_trace("unknown_tool_fails").await;
    assert!(
        result.failures.is_empty(),
        "expectation was Failure, should not report a trace failure: {:#?}",
        result.failures
    );
    assert_eq!(result.records.len(), 1);
    let record = &result.records[0];
    assert!(!record.success);
    assert!(
        record
            .error
            .as_deref()
            .unwrap_or("")
            .contains("not registered"),
        "unexpected error message: {:?}",
        record.error
    );
}

#[tokio::test]
async fn assertion_mix_reports_no_mismatches() {
    let (result, _db, _guard) = run_trace("assertion_mix").await;
    assert!(
        result.failures.is_empty(),
        "unexpected failures: {:#?}",
        result.failures
    );
    assert_eq!(result.records.len(), 3);
    assert!(result.records[0].success);
    assert!(result.records[1].success);
    assert!(!result.records[2].success);
}

/// Feeds the runner a programmatic trace whose declared expectation is
/// deliberately wrong, then asserts `TraceResult::failures` reports the
/// mismatch with the right index, tool name, and reason.
#[tokio::test]
async fn trace_failure_reports_mismatch_details() {
    use serde_json::json;
    use support::trace_runner::{TraceExpectation, TraceOperation};

    let (db, tools, _guard) = fixture().await;
    let runner = TraceRunner::new(Arc::clone(&tools), Arc::clone(&db), "trace-user");
    let trace = Trace {
        name: "force_mismatch".into(),
        operations: vec![TraceOperation {
            tool_name: "echo".into(),
            params: json!({"message": "actual-output"}),
            expected: TraceExpectation::Success {
                assertions: json!({"contains_text": "never-emitted"}),
            },
        }],
    };
    let result = runner.replay(&trace).await.expect("replay");

    assert_eq!(result.failures.len(), 1, "expected exactly one mismatch");
    let failure = &result.failures[0];
    assert_eq!(failure.operation_index, 0);
    assert_eq!(failure.tool_name, "echo");
    assert!(
        failure.reason.contains("contains_text mismatch"),
        "unexpected failure reason: {}",
        failure.reason
    );

    // Even though the assertion failed, the underlying tool still
    // succeeded; the ActionRecord reflects tool-level success, not
    // assertion-level success.
    assert_eq!(result.records.len(), 1);
    assert!(result.records[0].success);
}

#[tokio::test]
async fn action_records_are_persisted_to_db() {
    let (result, db, _guard) = run_trace("idempotency").await;
    let persisted = db
        .get_job_actions(result.job_id)
        .await
        .expect("get_job_actions");
    assert_eq!(
        persisted.len(),
        result.records.len(),
        "expected {} persisted actions, got {}",
        result.records.len(),
        persisted.len()
    );
    for (in_mem, stored) in result.records.iter().zip(persisted.iter()) {
        assert_eq!(in_mem.tool_name, stored.tool_name);
        assert_eq!(in_mem.sequence, stored.sequence);
        assert_eq!(in_mem.success, stored.success);
    }
}

/// Replaying the same trace twice against fresh DBs should produce
/// ActionRecords that match except for intentionally variable fields
/// (ids, wall-clock timestamps, measured durations).
#[tokio::test]
async fn trace_replay_is_deterministic_across_runs() {
    let (a, _db_a, _guard_a) = run_trace("echo_roundtrip").await;
    let (b, _db_b, _guard_b) = run_trace("echo_roundtrip").await;

    assert_eq!(a.records.len(), b.records.len());
    for (ra, rb) in a.records.iter().zip(b.records.iter()) {
        assert_eq!(ra.sequence, rb.sequence);
        assert_eq!(ra.tool_name, rb.tool_name);
        assert_eq!(ra.input, rb.input);
        assert_eq!(ra.success, rb.success);
        assert_eq!(ra.output_raw, rb.output_raw);
        assert_eq!(ra.output_sanitized, rb.output_sanitized);
        assert_eq!(ra.error, rb.error);
    }
}
