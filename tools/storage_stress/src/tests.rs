use super::*;

#[test]
fn redacts_postgres_uri_credentials_but_keeps_host() {
    let redacted = redact_postgres_url("postgres://user:secret@localhost:5432/app");

    assert_eq!(redacted, "postgres://<redacted>@localhost:5432/app");
    assert!(!redacted.contains("secret"));
}

#[test]
fn redacts_postgresql_uri_password_query_parameter() {
    let redacted =
        redact_postgres_url("postgresql://localhost/app?sslmode=require&password=secret");

    assert_eq!(
        redacted,
        "postgresql://localhost/app?sslmode=require&password=<redacted>"
    );
    assert!(!redacted.contains("secret"));
}

#[test]
fn redacts_key_value_postgres_password() {
    let redacted = redact_postgres_url("host=localhost user=postgres password=secret dbname=app");

    assert_eq!(
        redacted,
        "host=localhost user=postgres password=<redacted> dbname=app"
    );
    assert!(!redacted.contains("secret"));
}

#[test]
fn redacts_libsql_absolute_path() {
    let redacted = redact_libsql_path(Path::new("/tmp/ironclaw-storage-stress-secret.db"));

    assert_eq!(redacted, "libsql://<redacted-local-path>");
    assert!(!redacted.contains("/tmp"));
}

#[test]
fn synthetic_ids_are_generated_once_for_requested_cardinality() {
    let args = test_args();
    let ids = SyntheticIds::new(&args).expect("synthetic ids build");

    assert_eq!(ids.tenant_count(), args.tenants);
    assert_eq!(ids.user_count(), args.users);
}

#[test]
fn chat_turn_rejects_multi_process_runs() {
    let mut args = test_args();
    args.scenario = Scenario::ChatTurn;
    args.processes = 2;

    let error = validate_args(&args).expect_err("chat-turn is single-process only");

    assert!(error.contains("--scenario chat-turn requires --processes 1"));
}

#[test]
fn failure_causes_are_grouped_by_bucket_and_stage() {
    let samples = vec![
        Sample {
            latency: Duration::from_micros(10),
            error: Some("turn_thread_busy".to_string()),
            failure: Some(FailureCause::new(
                "turn_thread_busy",
                "submit_turn",
                "thread already has an active run",
            )),
            stages: None,
        },
        Sample {
            latency: Duration::from_micros(20),
            error: Some("turn_thread_busy".to_string()),
            failure: Some(FailureCause::new(
                "turn_thread_busy",
                "mark_rejected_busy",
                "ignored alternate detail",
            )),
            stages: None,
        },
    ];

    let causes = summarize_failure_causes(&samples);
    let busy = causes
        .get("turn_thread_busy")
        .expect("busy failure summary");

    assert_eq!(busy.count, 2);
    assert_eq!(busy.stages["submit_turn"], 1);
    assert_eq!(busy.stages["mark_rejected_busy"], 1);
    assert_eq!(busy.sample_detail, "thread already has an active run");
}

fn test_args() -> Args {
    Args {
        backend: Backend::Libsql,
        processes: 1,
        concurrency: 2,
        operations: 3,
        users: 4,
        tenants: 2,
        scenario: Scenario::ReserveRelease,
        run_id: None,
        libsql_path: None,
        postgres_url: None,
        postgres_pool_size: 4,
        progress_interval_seconds: 0,
        span_log_failures: false,
        slow_span_threshold_ms: 0,
        span_sample_limit: 100,
        child_index: None,
    }
}
