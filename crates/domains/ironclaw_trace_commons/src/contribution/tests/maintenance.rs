//! Queue compaction, quarantine, warning and telemetry accounting, and the worker tick.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::contribution::*;

use super::support::*;

#[tokio::test]
async fn queue_flush_compacts_duplicate_envelopes_and_orphan_holds_before_submit() {
    let scope = format!("trace-queue-compaction-test-{}", Uuid::new_v4());
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
        .set_bearer_token_env("TRACE_COMMONS_MISSING_TOKEN".to_string())
        .set_auto_submit_high_value_traces(true)
        .set_min_submission_score(0.0);
    write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut older = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut older);
    older.created_at = Utc::now() - chrono::Duration::minutes(5);
    let older_path = queue_trace_envelope_for_scope(Some(&scope), &older).expect("older queued");

    let mut newer = older.clone();
    newer.submission_id = Uuid::new_v4();
    newer.created_at = Utc::now();
    let newer_path = queue_trace_envelope_for_scope(Some(&scope), &newer).expect("newer queued");

    let orphan_id = Uuid::new_v4();
    let orphan_path = trace_queue_dir(Some(&scope)).join(format!("{orphan_id}.held.json"));
    std::fs::write(
        &orphan_path,
        serde_json::json!({ "reason": "old sidecar" }).to_string(),
    )
    .expect("orphan hold writes");

    let report = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
        .await
        .expect("flush handles retryable submit failure after compaction");

    assert_eq!(report.compaction.duplicate_envelopes_removed, 1);
    assert_eq!(report.compaction.orphan_hold_sidecars_removed, 1);
    assert!(!older_path.exists(), "older duplicate should be removed");
    assert!(newer_path.exists(), "newest duplicate should remain queued");
    assert!(
        !orphan_path.exists(),
        "orphan hold sidecar should be removed"
    );

    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    assert_eq!(diagnostics.queued_count, 1);
    assert_eq!(
        diagnostics
            .telemetry
            .last_compaction
            .as_ref()
            .expect("last compaction is recorded")
            .duplicate_envelopes_removed,
        1
    );
    assert_eq!(diagnostics.telemetry.compaction_reclaimed_items_total, 2);

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_flush_quarantines_malformed_envelope_and_submits_later_valid_envelope() {
    let scope = format!("trace-queue-malformed-recovery-test-{}", Uuid::new_v4());
    let submitted_ids = Arc::new(std::sync::Mutex::new(Vec::<Uuid>::new()));
    let submitted_ids_for_route = submitted_ids.clone();
    let app = axum::Router::new()
        .route(
            "/v1/traces",
            axum::routing::post(
                move |axum::Json(body): axum::Json<TraceContributionEnvelope>| {
                    let submitted_ids = submitted_ids_for_route.clone();
                    async move {
                        submitted_ids
                            .lock()
                            .expect("submitted ids lock")
                            .push(body.submission_id);
                        axum::Json(serde_json::json!({
                            "status": "accepted",
                            "credit_points_pending": 1.0,
                            "explanation": ["accepted"]
                        }))
                    }
                },
            ),
        )
        .route(
            "/v1/contributors/me/submission-status",
            axum::routing::post(|| async { axum::Json(Vec::<TraceSubmissionStatusUpdate>::new()) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock trace commons listener binds");
    let endpoint = format!(
        "http://{}/v1/traces",
        listener.local_addr().expect("local addr")
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint(endpoint)
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0),
    )
    .expect("policy writes");

    let queue_dir = trace_queue_dir(Some(&scope));
    std::fs::create_dir_all(&queue_dir).expect("queue dir exists");
    let malformed_path = queue_dir.join(format!("{}.json", Uuid::nil()));
    let malformed_body = r#"{"redacted_content":"[REDACTED local-only body]","#;
    std::fs::write(&malformed_path, malformed_body).expect("malformed fixture writes");

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);
    queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("valid envelope queues");

    let provider = RefreshingTestUploadCredentialProvider::new("trace-token", "trace-token");
    let report = flush_trace_contribution_queue_for_scope_with_credential_provider(
        Some(&scope),
        10,
        &provider,
    )
    .await
    .expect("flush should skip malformed envelope and submit valid envelope");

    assert_eq!(report.submitted, 1);
    assert_eq!(report.compaction.malformed_envelopes_quarantined, 1);
    assert!(
        !malformed_path.exists(),
        "malformed envelope should leave active queue"
    );
    let quarantine_path =
        trace_queue_malformed_dir(Some(&scope)).join(format!("{}.json", Uuid::nil()));
    assert!(
        quarantine_path.exists(),
        "malformed envelope should be quarantined"
    );
    assert_eq!(
        std::fs::read_to_string(&quarantine_path).expect("quarantine body reads"),
        malformed_body
    );
    assert_eq!(
        *submitted_ids.lock().expect("submitted ids lock"),
        vec![envelope.submission_id]
    );
    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    assert_eq!(diagnostics.queued_count, 0);

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_compaction_keeps_same_trace_when_semantic_metadata_differs() {
    let scope = format!("trace-queue-exact-compaction-test-{}", Uuid::new_v4());
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
        .set_bearer_token_env("TRACE_COMMONS_MISSING_TOKEN".to_string())
        .set_auto_submit_high_value_traces(true)
        .set_min_submission_score(0.0);
    write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut base = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut base);
    base.created_at = Utc::now() - chrono::Duration::minutes(5);
    let base_path = queue_trace_envelope_for_scope(Some(&scope), &base).expect("base queued");

    let mut changed = base.clone();
    changed.submission_id = Uuid::new_v4();
    changed.created_at = Utc::now();
    changed.outcome.task_success = TaskSuccess::Failure;
    changed
        .value_card
        .limitations
        .push("Different replay utility metadata.".to_string());
    changed.trace_card.redaction_pipeline_version = "legacy-trace-card-redactor".to_string();
    let changed_path =
        queue_trace_envelope_for_scope(Some(&scope), &changed).expect("changed queued");

    let report = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
        .await
        .expect("flush handles retryable submit failures");

    assert_eq!(report.compaction.duplicate_envelopes_removed, 0);
    assert!(base_path.exists(), "base envelope should remain queued");
    assert!(
        changed_path.exists(),
        "semantically different envelope should remain queued"
    );

    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    assert_eq!(diagnostics.queued_count, 2);
    assert!(
        diagnostics.warnings.iter().any(|warning| {
            warning.kind == TraceQueueWarningKind::TraceCardRedactionPipelineMismatch
        }),
        "warning from changed envelope should not be hidden by compaction"
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_compaction_failure_records_sanitized_queue_telemetry() {
    let scope = format!("trace-queue-compaction-failure-test-{}", Uuid::new_v4());
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
        .set_bearer_token_env("TRACE_COMMONS_MISSING_TOKEN".to_string())
        .set_auto_submit_high_value_traces(true)
        .set_min_submission_score(0.0);
    write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut older = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut older);
    older.created_at = Utc::now() - chrono::Duration::minutes(5);
    let older_path = queue_trace_envelope_for_scope(Some(&scope), &older).expect("older queued");

    let mut newer = older.clone();
    newer.submission_id = Uuid::new_v4();
    newer.created_at = Utc::now();
    let _newer_path = queue_trace_envelope_for_scope(Some(&scope), &newer).expect("newer queued");

    let older_hold_path = trace_queue_hold_path_for_envelope_path(&older_path);
    std::fs::create_dir_all(&older_hold_path).expect("hold directory fixture creates");

    let error = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
        .await
        .expect_err("compaction hold removal failure should fail flush");
    // #7144: the unreadable sidecar used to be swallowed by
    // `.ok().flatten()`, so compaction ranked the held envelope as unheld
    // and only tripped later, on the duplicate-hold removal. It now fails at
    // the read — earlier, and naming the artifact that could not be read
    // instead of a downstream symptom.
    let message = error.to_string();
    assert!(
        message.contains("hold sidecar"),
        "the failure must name the unreadable hold, got: {message}"
    );

    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    let failure = diagnostics
        .telemetry
        .last_failure
        .as_ref()
        .expect("compaction failure is recorded");
    assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::Queue);
    assert!(failure.reason.contains("flush failed"));
    assert!(!failure.reason.contains(&scope));
    assert!(!failure.reason.contains("TRACE_COMMONS_MISSING_TOKEN"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_diagnostics_reports_schema_policy_and_redaction_warnings() {
    let scope = format!("trace-queue-warning-test-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string()),
    )
    .expect("policy writes");

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    envelope.schema_version = "ironclaw.trace_contribution.v0".to_string();
    envelope.consent.policy_version = "2025-01-01".to_string();
    envelope.privacy.redaction_pipeline_version = "legacy-redactor".to_string();
    envelope.trace_card.redaction_pipeline_version = "legacy-redactor".to_string();
    queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    assert!(
        diagnostics
            .warnings
            .iter()
            .any(|warning| warning.kind == TraceQueueWarningKind::SchemaVersionMismatch)
    );
    assert!(
        diagnostics
            .warnings
            .iter()
            .any(|warning| warning.kind == TraceQueueWarningKind::PolicyVersionMismatch)
    );
    assert!(
        diagnostics
            .warnings
            .iter()
            .any(|warning| warning.kind == TraceQueueWarningKind::RedactionPipelineMismatch)
    );
    assert!(
        diagnostics
            .warnings
            .iter()
            .all(|warning| warning.promotion_blocking)
    );
    assert!(
        diagnostics
            .warnings
            .iter()
            .all(|warning| !warning.recommended_action.trim().is_empty())
    );
    let serialized = serde_json::to_string(&diagnostics).expect("diagnostics serialize");
    assert!(!serialized.contains("legacy-redactor"));
    assert!(!serialized.contains("2025-01-01"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_telemetry_classifies_endpoint_credential_and_network_failures() {
    let endpoint_scope = format!("trace-queue-endpoint-classification-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&endpoint_scope),
        &StandingTraceContributionPolicy::default().set_enabled(true),
    )
    .expect("endpoint policy writes");
    let endpoint_result = flush_trace_contribution_queue_for_scope(Some(&endpoint_scope), 10).await;
    assert!(endpoint_result.is_err());
    let endpoint_diagnostics =
        trace_queue_diagnostics_for_scope(Some(&endpoint_scope)).expect("diagnostics");
    assert_eq!(
        endpoint_diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("endpoint failure recorded")
            .kind,
        TraceQueueTelemetryFailureKind::Endpoint
    );

    let credential_scope = format!("trace-queue-credential-classification-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&credential_scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
            .set_bearer_token_env("TRACE_COMMONS_MISSING_TOKEN".to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0),
    )
    .expect("credential policy writes");
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);
    queue_trace_envelope_for_scope(Some(&credential_scope), &envelope)
        .expect("queued envelope writes");
    flush_trace_contribution_queue_for_scope(Some(&credential_scope), 10)
        .await
        .expect("credential submission failure is held for retry");
    let credential_diagnostics =
        trace_queue_diagnostics_for_scope(Some(&credential_scope)).expect("diagnostics");
    assert_eq!(
        credential_diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("credential failure recorded")
            .kind,
        TraceQueueTelemetryFailureKind::Credential
    );

    let network_scope = format!("trace-queue-network-classification-{}", Uuid::new_v4());
    let token_env = "TRACE_COMMONS_QUEUE_NETWORK_CLASSIFICATION_TOKEN";
    let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
    write_trace_policy_for_scope(
        Some(&network_scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
            .set_bearer_token_env(token_env.to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0),
    )
    .expect("network policy writes");
    let mut envelope = envelope.clone();
    envelope.submission_id = Uuid::new_v4();
    queue_trace_envelope_for_scope(Some(&network_scope), &envelope)
        .expect("queued envelope writes");
    flush_trace_contribution_queue_for_scope(Some(&network_scope), 10)
        .await
        .expect("network submission failure is held for retry");
    let network_diagnostics =
        trace_queue_diagnostics_for_scope(Some(&network_scope)).expect("diagnostics");
    assert_eq!(
        network_diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("network failure recorded")
            .kind,
        TraceQueueTelemetryFailureKind::NetworkConnectionRefused
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&endpoint_scope)));
    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&credential_scope)));
    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&network_scope)));
}
#[test]
fn queue_telemetry_classifies_network_subtypes_without_raw_error_details() {
    let now = Utc::now();
    let cases = [
        (
            anyhow::anyhow!(
                "request failed: DNS lookup failed for https://private.example/v1/traces"
            ),
            TraceQueueTelemetryFailureKind::NetworkDns,
        ),
        (
            anyhow::anyhow!("request failed: operation timed out contacting trace service"),
            TraceQueueTelemetryFailureKind::NetworkTimeout,
        ),
        (
            anyhow::anyhow!("request failed: connection refused by 127.0.0.1:9"),
            TraceQueueTelemetryFailureKind::NetworkConnectionRefused,
        ),
        (
            anyhow::anyhow!("request failed: network is unreachable while offline"),
            TraceQueueTelemetryFailureKind::NetworkOffline,
        ),
    ];

    for (error, expected) in cases {
        let failure =
            trace_queue_telemetry_failure_with_label(&error, now, "submission retry scheduled");
        assert_eq!(failure.kind, expected);
        assert!(failure.reason.contains("error_hash="));
        assert!(!failure.reason.contains("private.example"));
        assert!(!failure.reason.contains("127.0.0.1"));
    }
}
#[test]
fn queue_telemetry_classifies_typed_llm_auth_and_session_failures_without_raw_details() {
    let now = Utc::now();
    let cases = [
        (
            anyhow::Error::from(ironclaw_llm::error::LlmError::AuthFailed {
                provider: "trace-secret-provider".to_string(),
            }),
            TraceQueueTelemetryFailureKind::Credential,
        ),
        (
            anyhow::Error::from(ironclaw_llm::error::LlmError::SessionExpired {
                provider: "trace-secret-provider".to_string(),
            }),
            TraceQueueTelemetryFailureKind::Credential,
        ),
    ];

    for (error, expected) in cases {
        let failure =
            trace_queue_telemetry_failure_with_label(&error, now, "provider boundary failed");
        assert_eq!(failure.kind, expected);
        assert!(failure.reason.contains("error_hash="));
        assert!(!failure.reason.contains("trace-secret-provider"));
    }
}
#[tokio::test]
async fn trace_queue_worker_tick_flushes_scopes_and_returns_credit_notices_for_delivery() {
    let scope = format!("trace-worker-tick-test-{}", Uuid::new_v4());
    let token_env = "TRACE_COMMONS_WORKER_TICK_TEST_TOKEN";
    let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
        .set_bearer_token_env(token_env.to_string())
        .set_auto_submit_high_value_traces(true)
        .set_min_submission_score(0.0)
        .set_credit_notice_interval_hours(168);
    write_trace_policy_for_scope(Some(&scope), &policy).expect("policy writes");

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);
    queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: Some(1.5),
            credit_explanation: vec!["Delayed utility credit posted.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    let report = flush_trace_contribution_queue_worker_tick(vec![scope.clone()], 10)
        .await
        .expect("worker tick succeeds");

    assert_eq!(report.scopes_checked, 1);
    assert_eq!(report.submitted, 0);
    assert_eq!(report.held, 1);
    assert_eq!(report.scope_reports[0].scope, scope);
    let notice = report.scope_reports[0]
        .credit_notice
        .as_ref()
        .expect("worker returns due credit notice for caller delivery");
    assert_eq!(notice.pending_credit, 1.0);
    assert_eq!(notice.final_credit, 1.5);

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert!(
        records[0].last_credit_notice_at.is_some(),
        "worker tick marks due notices only when it returns them for delivery"
    );
}
#[tokio::test]
async fn trace_queue_worker_tick_records_durable_failure_and_success_telemetry() {
    let scope = format!("trace-worker-telemetry-test-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default().set_enabled(true),
    )
    .expect("failure policy writes");

    let failed = flush_trace_contribution_queue_worker_tick(vec![scope.clone()], 10)
        .await
        .expect("worker tick handles scoped failure");
    assert_eq!(failed.scopes_checked, 1);
    assert!(failed.scope_reports.is_empty());

    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    assert!(diagnostics.telemetry.last_flush_attempt_at.is_some());
    assert!(diagnostics.telemetry.last_failed_flush_at.is_some());
    assert_eq!(diagnostics.telemetry.consecutive_flush_failures, 1);
    let failure = diagnostics
        .telemetry
        .last_failure
        .as_ref()
        .expect("failure metadata is stored");
    assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::Endpoint);
    assert!(failure.reason.contains("flush failed"));
    assert!(!failure.reason.contains(&scope));

    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string()),
    )
    .expect("success policy writes");

    let succeeded = flush_trace_contribution_queue_worker_tick(vec![scope.clone()], 10)
        .await
        .expect("worker tick handles scoped success");
    assert_eq!(succeeded.scopes_checked, 1);
    assert_eq!(succeeded.scope_reports.len(), 1);

    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    assert!(diagnostics.telemetry.last_successful_flush_at.is_some());
    assert_eq!(diagnostics.telemetry.consecutive_flush_failures, 0);
    assert!(diagnostics.telemetry.last_failure.is_none());

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn read_trace_queue_holds_for_scope_returns_sidecars_without_envelope_bodies() {
    let scope = format!("trace-queue-holds-test-{}", Uuid::new_v4());
    let dir = trace_queue_dir(Some(&scope));
    std::fs::create_dir_all(&dir).expect("queue dir exists");
    let submission_id = Uuid::new_v4();
    let queue_path = dir.join(format!("{submission_id}.json"));
    std::fs::write(&queue_path, "raw envelope body should not be exposed")
        .expect("queued envelope fixture writes");
    write_trace_queue_hold_reason(&queue_path, "requires manual review")
        .expect("hold reason writes");

    std::fs::write(
        dir.join(format!("{}.held.json", Uuid::new_v4())),
        "{not-json",
    )
    .expect("malformed hold fixture writes");
    std::fs::write(
        dir.join("not-a-submission.held.json"),
        serde_json::json!({ "reason": "should be ignored" }).to_string(),
    )
    .expect("invalid id hold fixture writes");

    let holds = read_trace_queue_holds_for_scope(Some(&scope)).expect("holds read");

    assert_eq!(holds.len(), 1);
    assert_eq!(holds[0].submission_id, submission_id);
    assert_eq!(holds[0].reason, "requires manual review");
    let serialized = serde_json::to_string(&holds).expect("holds serialize");
    assert!(!serialized.contains("raw envelope body should not be exposed"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn server_rescrub_redacts_late_leaks_before_storage() {
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default()
            .set_include_message_text(true)
            .set_include_tool_payloads(true),
    );
    let mut envelope = DeterministicTraceRedactor::with_known_path_prefixes([PathBuf::from(
        "/Users/alice/project",
    )])
    .redact_trace(raw)
    .await
    .expect("redaction should succeed");

    envelope.events[0].redacted_content =
        Some("late leak at /tmp/ironclaw/private/token.txt".to_string());
    envelope.events[1].structured_payload = serde_json::json!({
        "Authorization": "Bearer abcdefghijklmnopqrstuvwxyz123456",
        "path": "/tmp/ironclaw/private/token.txt"
    });
    rescrub_trace_envelope_with(&DeterministicTraceRedactor::new(Vec::new()), &mut envelope)
        .expect("re-scrub should succeed");

    let json = serde_json::to_string(&envelope).expect("envelope serializes");
    assert!(json.contains("<PRIVATE_LOCAL_PATH_"));
    assert!(json.contains(SERVER_RESCRUB_PIPELINE_SUFFIX));
    assert!(!json.contains("/tmp/ironclaw/private/token.txt"));
    assert!(!json.contains("abcdefghijklmnopqrstuvwxyz"));
    assert!(
        envelope
            .privacy
            .redaction_counts
            .get("local_path")
            .copied()
            .unwrap_or_default()
            >= 3
    );
}
