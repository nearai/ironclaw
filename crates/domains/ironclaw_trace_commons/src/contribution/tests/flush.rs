//! Manual-review holds and the queue-flush paths, including their failure classification.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::contribution::*;

use super::support::*;

#[tokio::test]
async fn queue_trace_envelope_as_held_retains_envelope_and_manual_review_sidecar() {
    // A held (e.g. PII-gated) trace must be retained for review, not
    // dropped: the envelope is queued AND a ManualReview hold sidecar is
    // written so the flush worker skips it until it is authorized.
    let scope = format!("trace-held-retain-test-{}", Uuid::new_v4());
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);

    let reason = "manual review required because residual privacy risk is high";
    let queue_path = queue_trace_envelope_as_held_for_scope(Some(&scope), &envelope, reason)
        .expect("held envelope queues");

    assert!(
        queue_path.exists(),
        "held envelope must be retained on disk"
    );

    let holds = read_trace_queue_holds_for_scope(Some(&scope)).expect("read holds");
    assert_eq!(holds.len(), 1, "exactly one hold sidecar");
    assert_eq!(holds[0].submission_id, envelope.submission_id);
    assert_eq!(holds[0].kind, TraceQueueHoldKind::ManualReview);
    assert!(
        holds[0].reason.contains("residual privacy risk is high"),
        "hold reason preserved, got {}",
        holds[0].reason
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[test]
fn retain_manual_review_holds_excludes_policy_and_retry_holds() {
    let holds = vec![
        TraceQueueHold {
            submission_id: Uuid::new_v4(),
            kind: TraceQueueHoldKind::ManualReview,
            reason: "residual privacy risk is high".to_string(),
            attempts: 0,
            next_retry_at: None,
        },
        TraceQueueHold {
            submission_id: Uuid::new_v4(),
            kind: TraceQueueHoldKind::PolicyGate,
            reason: "submission score below minimum".to_string(),
            attempts: 0,
            next_retry_at: None,
        },
        TraceQueueHold {
            submission_id: Uuid::new_v4(),
            kind: TraceQueueHoldKind::RetryableSubmissionFailure,
            reason: "retained for retry".to_string(),
            attempts: 1,
            next_retry_at: None,
        },
    ];
    let kept = retain_manual_review_holds(holds);
    assert_eq!(kept.len(), 1, "only the ManualReview hold is surfaced");
    assert_eq!(kept[0].kind, TraceQueueHoldKind::ManualReview);
}
#[tokio::test]
async fn authorize_manual_review_hold_promotes_envelope_past_all_gates() {
    let scope = format!("trace-authorize-test-{}", Uuid::new_v4());
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::High;
    apply_credit_estimate_to_envelope(&mut envelope);

    let manual_policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_require_manual_approval_when_pii_detected(true);

    // Precondition: the High-PII trace is held for manual review.
    queue_trace_envelope_as_held_for_scope(
        Some(&scope),
        &envelope,
        "manual review required because residual privacy risk is high",
    )
    .expect("held envelope queues");
    assert_eq!(
        manual_review_holds_for_scope(Some(&scope)).unwrap().len(),
        1
    );
    assert!(matches!(
        trace_autonomous_eligibility(&envelope, &manual_policy),
        TraceQueueEligibility::Hold {
            kind: TraceQueueHoldKind::ManualReview,
            ..
        }
    ));

    // Authorize -> promotes as-is.
    let authorized = authorize_manual_review_hold_for_scope(Some(&scope), envelope.submission_id)
        .expect("authorize succeeds");
    assert!(authorized, "the held trace is authorized");

    // Hold cleared, envelope stamped, eligibility now submits.
    assert!(
        manual_review_holds_for_scope(Some(&scope))
            .unwrap()
            .is_empty(),
        "hold sidecar removed"
    );
    let reloaded_path =
        trace_queue_dir(Some(&scope)).join(format!("{}.json", envelope.submission_id));
    let reloaded = load_trace_envelope(&reloaded_path).expect("reload envelope");
    assert!(
        reloaded.manual_review_authorized,
        "envelope stamped authorized"
    );
    assert!(
        matches!(
            trace_autonomous_eligibility(&reloaded, &manual_policy),
            TraceQueueEligibility::Submit
        ),
        "authorized trace now submits despite High PII"
    );

    // Authorizing an unknown submission is a no-op, not an error.
    assert!(
        !authorize_manual_review_hold_for_scope(Some(&scope), Uuid::new_v4())
            .expect("unknown submission is Ok(false)")
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn manual_review_holds_for_scope_returns_only_manual_review_holds() {
    let scope = format!("trace-manual-holds-test-{}", Uuid::new_v4());
    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);

    queue_trace_envelope_as_held_for_scope(
        Some(&scope),
        &envelope,
        "manual review required because residual privacy risk is high",
    )
    .expect("held envelope queues");

    let holds = manual_review_holds_for_scope(Some(&scope)).expect("read manual-review holds");
    assert_eq!(holds.len(), 1, "the one ManualReview hold is returned");
    assert_eq!(holds[0].submission_id, envelope.submission_id);
    assert_eq!(holds[0].kind, TraceQueueHoldKind::ManualReview);
    assert!(holds[0].reason.contains("residual privacy risk is high"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_flush_holds_failed_submission_and_still_returns_due_credit_notice() {
    let scope = format!("trace-flush-submit-failure-test-{}", Uuid::new_v4());
    let token_env = "TRACE_COMMONS_FLUSH_HOLD_TEST_TOKEN";
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
    let queue_path =
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
            credit_points_pending: 1.5,
            credit_points_final: Some(2.5),
            credit_explanation: vec!["Delayed utility credit posted.".to_string()],
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    let report = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
        .await
        .expect("flush should not abort on one failed submission");

    assert_eq!(report.submitted, 0);
    assert_eq!(report.held, 1);
    assert_eq!(report.holds[0].submission_id, envelope.submission_id);
    assert!(queue_path.exists(), "failed envelope should stay queued");
    assert!(report.holds[0].reason.contains("retained for retry"));
    assert!(!report.holds[0].reason.contains("127.0.0.1"));
    assert!(!report.holds[0].reason.contains("super-secret-token"));

    let hold_path = queue_path.with_extension("held.json");
    let hold_body = std::fs::read_to_string(&hold_path).expect("hold reason writes");
    assert!(hold_body.contains("retained for retry"));
    assert!(!hold_body.contains("127.0.0.1"));
    assert!(!hold_body.contains("super-secret-token"));

    let notice = report
        .credit_notice
        .expect("due credit notice should still be evaluated");
    assert_eq!(notice.submissions_submitted, 1);
    assert_eq!(notice.pending_credit, 1.5);
    assert_eq!(notice.final_credit, 2.5);
}
#[tokio::test]
async fn queue_flush_records_typed_retry_state_and_defers_until_backoff_expires() {
    let scope = format!("trace-flush-typed-retry-state-test-{}", Uuid::new_v4());
    let token_env = "TRACE_COMMONS_TYPED_RETRY_TEST_TOKEN";
    let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
        .set_bearer_token_env(token_env.to_string())
        .set_auto_submit_high_value_traces(true)
        .set_min_submission_score(0.0);
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
    let queue_path =
        queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

    let first = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
        .await
        .expect("first flush should hold failed submission");
    assert_eq!(first.submitted, 0);
    assert_eq!(first.held, 1);
    assert_eq!(
        first.holds[0].kind,
        TraceQueueHoldKind::RetryableSubmissionFailure
    );
    assert_eq!(first.holds[0].attempts, 1);
    let first_retry_at = first.holds[0]
        .next_retry_at
        .expect("retry failure gets a next retry time");
    assert!(first_retry_at > Utc::now());

    let second = flush_trace_contribution_queue_for_scope(Some(&scope), 10)
        .await
        .expect("second flush should respect retry backoff");
    assert_eq!(second.submitted, 0);
    assert_eq!(second.held, 1);
    assert_eq!(
        second.holds[0].kind,
        TraceQueueHoldKind::RetryableSubmissionFailure
    );
    assert_eq!(
        second.holds[0].attempts, 1,
        "a backoff-held envelope must not consume another retry attempt"
    );
    assert_eq!(second.holds[0].next_retry_at, Some(first_retry_at));

    let holds = read_trace_queue_holds_for_scope(Some(&scope)).expect("holds read");
    assert_eq!(
        holds[0].kind,
        TraceQueueHoldKind::RetryableSubmissionFailure
    );
    assert_eq!(holds[0].attempts, 1);
    assert_eq!(holds[0].next_retry_at, Some(first_retry_at));

    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    assert_eq!(diagnostics.retry_scheduled_count, 1);
    assert_eq!(diagnostics.manual_review_hold_count, 0);
    assert_eq!(diagnostics.policy_hold_count, 0);
    assert_eq!(diagnostics.next_retry_at, Some(first_retry_at));

    let hold_body = std::fs::read_to_string(queue_path.with_extension("held.json"))
        .expect("hold reason writes");
    assert!(hold_body.contains("\"kind\": \"retryable_submission_failure\""));
    assert!(hold_body.contains("\"attempts\": 1"));
    assert!(!hold_body.contains("127.0.0.1"));
    assert!(!hold_body.contains("super-secret-token"));
}
#[tokio::test]
async fn queue_flush_uses_refreshed_upload_claim_for_submit_and_status_sync() {
    let scope = format!("trace-issuer-refresh-test-{}", Uuid::new_v4());
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen_for_submit = seen.clone();
    let seen_for_status = seen.clone();
    let app = axum::Router::new()
        .route(
            "/v1/traces",
            axum::routing::post(
                move |headers: axum::http::HeaderMap,
                      axum::Json(_body): axum::Json<TraceContributionEnvelope>| {
                    let seen = seen_for_submit.clone();
                    async move {
                        let authorization = headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or("<missing>")
                            .to_string();
                        seen.lock().expect("seen lock").push(authorization.clone());
                        if authorization == "Bearer stale-upload-claim" {
                            return (
                                axum::http::StatusCode::UNAUTHORIZED,
                                axum::Json(serde_json::json!({"error": "expired"})),
                            );
                        }
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(serde_json::json!({
                                "status": "accepted",
                                "credit_points_pending": 1.0,
                                "explanation": ["accepted"]
                            })),
                        )
                    }
                },
            ),
        )
        .route(
            "/v1/contributors/me/submission-status",
            axum::routing::post(move |headers: axum::http::HeaderMap| {
                let seen = seen_for_status.clone();
                async move {
                    let authorization = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("<missing>")
                        .to_string();
                    seen.lock().expect("seen lock").push(authorization);
                    axum::Json(Vec::<TraceSubmissionStatusUpdate>::new())
                }
            }),
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

    let provider =
        RefreshingTestUploadCredentialProvider::new("stale-upload-claim", "fresh-upload-claim");
    let report = flush_trace_contribution_queue_for_scope_with_credential_provider(
        Some(&scope),
        10,
        &provider,
    )
    .await
    .expect("flush retries with refreshed claim");

    assert_eq!(report.submitted, 1);
    assert_eq!(report.held, 0);
    assert_eq!(
        *seen.lock().expect("seen lock"),
        vec![
            "Bearer stale-upload-claim".to_string(),
            "Bearer fresh-upload-claim".to_string(),
            "Bearer fresh-upload-claim".to_string()
        ]
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
/// Regression: an instance-only-enrolled scope has NO enabled per-scope
/// policy — its policy, device key, and per-user subject come from the
/// resolved effective flush target. Status sync must run off that resolved
/// target instead of re-reading the per-scope policy (which would silently
/// return Ok(0) right after a successful instance-attributed submission,
/// so final credit status never lands locally).
#[tokio::test]
async fn status_sync_with_target_uses_resolved_instance_credential_context() {
    let scope = format!("trace-instance-status-sync-test-{}", Uuid::new_v4());

    // Seed a Submitted record for the scope. Deliberately do NOT write a
    // per-scope policy: the old per-scope re-read would bail with Ok(0).
    let record = submitted_credit_record(1.0, None, None, Vec::new());
    let submission_id = record.submission_id;
    let trace_id = record.trace_id;
    write_local_trace_records_for_scope(Some(&scope), &[record]).expect("record writes");

    let app = axum::Router::new().route(
        "/v1/contributors/me/submission-status",
        axum::routing::post(move || async move {
            axum::Json(vec![TraceSubmissionStatusUpdate {
                submission_id,
                trace_id,
                status: "accepted".to_string(),
                credit_points_pending: 1.0,
                credit_points_final: Some(2.0),
                credit_points_ledger: 0.0,
                credit_points_total: Some(2.0),
                explanation: Vec::new(),
                delayed_credit_explanations: Vec::new(),
            }])
        }),
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

    let instance_policy = StandingTraceContributionPolicy {
        enabled: true,
        ingestion_endpoint: Some(endpoint),
        ..Default::default()
    };
    let instance_dir = tempfile::tempdir().expect("instance device-key dir");
    let provider = CapturingUploadCredentialProvider::default();

    let synced = sync_remote_trace_submission_records_for_scope_unlocked_with_target(
        Some(&scope),
        &instance_policy,
        instance_dir.path(),
        Some("subject-abc"),
        &provider,
    )
    .await
    .expect("instance-target status sync succeeds");
    assert_eq!(synced, 1, "the submitted record must sync its final status");

    let contexts = provider.contexts.lock().expect("contexts lock");
    assert_eq!(contexts.len(), 1, "one bearer mint for one status chunk");
    assert_eq!(
        contexts[0].0.as_deref(),
        Some("subject-abc"),
        "claim context must carry the resolved per-user subject"
    );
    assert_eq!(
        contexts[0].1.as_deref(),
        Some(instance_dir.path()),
        "claim context must use the resolved instance device-key dir"
    );
    drop(contexts);

    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert_eq!(
        records[0].credit_points_final,
        Some(2.0),
        "final credit from the remote update must land on the local record"
    );

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_flush_classifies_upload_http_rejection_through_submit_call_site() {
    let scope = format!("trace-upload-http-classification-test-{}", Uuid::new_v4());
    let token_env = "TRACE_COMMONS_UPLOAD_HTTP_CLASSIFICATION_TOKEN";
    let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
    let app = axum::Router::new().route(
        "/v1/traces",
        axum::routing::post(|| async {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": "token expired"})),
            )
        }),
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
            .set_bearer_token_env(token_env.to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0),
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
    apply_credit_estimate_to_envelope(&mut envelope);
    queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

    flush_trace_contribution_queue_for_scope(Some(&scope), 10)
        .await
        .expect("upload HTTP rejection is held for retry");
    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    let failure = diagnostics
        .telemetry
        .last_failure
        .as_ref()
        .expect("upload rejection recorded");
    assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::HttpRejection);
    assert!(failure.reason.contains("error_hash="));
    assert!(!failure.reason.contains("token expired"));
    assert!(!failure.reason.contains("super-secret-token"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_flush_classifies_status_sync_request_failure_through_call_site() {
    let scope = format!("trace-status-sync-classification-test-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0),
    )
    .expect("policy writes");
    write_local_trace_records_for_scope(
        Some(&scope),
        &[submitted_credit_record(1.0, None, None, Vec::new())],
    )
    .expect("local record writes");

    flush_trace_contribution_queue_for_scope_with_credential_provider(
        Some(&scope),
        10,
        &RefreshingTestUploadCredentialProvider::new("super-secret-token", "super-secret-token"),
    )
    .await
    .expect("status sync failure is nonfatal during flush");
    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    let failure = diagnostics
        .telemetry
        .last_failure
        .as_ref()
        .expect("status sync failure recorded");
    assert_eq!(
        failure.kind,
        TraceQueueTelemetryFailureKind::NetworkConnectionRefused
    );
    assert!(failure.reason.contains("status sync failed"));
    assert!(!failure.reason.contains("127.0.0.1"));
    assert!(!failure.reason.contains("super-secret-token"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_flush_classifies_status_sync_auth_rejection_as_credential() {
    let scope = format!(
        "trace-status-sync-auth-classification-test-{}",
        Uuid::new_v4()
    );
    let token_env = "TRACE_COMMONS_STATUS_SYNC_AUTH_CLASSIFICATION_TOKEN";
    let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
    let app = axum::Router::new().route(
        "/v1/contributors/me/submission-status",
        axum::routing::post(|| async {
            (
                axum::http::StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({"error": "not authorized"})),
            )
        }),
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
            .set_bearer_token_env(token_env.to_string())
            .set_auto_submit_high_value_traces(true)
            .set_min_submission_score(0.0),
    )
    .expect("policy writes");
    write_local_trace_records_for_scope(
        Some(&scope),
        &[submitted_credit_record(1.0, None, None, Vec::new())],
    )
    .expect("local record writes");

    flush_trace_contribution_queue_for_scope(Some(&scope), 10)
        .await
        .expect("status sync auth failure is nonfatal during flush");
    let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
    let failure = diagnostics
        .telemetry
        .last_failure
        .as_ref()
        .expect("status sync auth failure recorded");
    assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::Credential);
    assert!(failure.reason.contains("status sync failed"));
    assert!(!failure.reason.contains("super-secret-token"));

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn queue_flush_classifies_typed_submission_provider_connection_loss_before_text() {
    let cases = [
        std::io::ErrorKind::ConnectionReset,
        std::io::ErrorKind::ConnectionAborted,
    ];

    for io_kind in cases {
        let scope = format!(
            "trace-submission-provider-connection-loss-classification-test-{}",
            Uuid::new_v4()
        );
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("https://trace.example.com/v1/traces".to_string())
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
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
        apply_credit_estimate_to_envelope(&mut envelope);
        queue_trace_envelope_for_scope(Some(&scope), &envelope).expect("queued envelope writes");

        let report = flush_trace_contribution_queue_for_scope_with_credential_provider(
            Some(&scope),
            10,
            &FailingTestUploadCredentialProvider { kind: io_kind },
        )
        .await
        .expect("submission provider failure is held for retry");
        assert_eq!(report.submitted, 0);
        assert_eq!(report.held, 1);

        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        let failure = diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("submission provider failure recorded");
        assert_eq!(failure.kind, TraceQueueTelemetryFailureKind::Network);
        assert!(failure.reason.contains("submission retry scheduled"));
        assert!(failure.reason.contains("error_hash="));
        assert!(!failure.reason.contains("super-secret-token"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }
}
#[tokio::test]
async fn queue_flush_classifies_typed_status_sync_provider_io_errors_before_text() {
    let cases = [
        (
            std::io::ErrorKind::TimedOut,
            TraceQueueTelemetryFailureKind::NetworkTimeout,
        ),
        (
            std::io::ErrorKind::NotConnected,
            TraceQueueTelemetryFailureKind::NetworkOffline,
        ),
        (
            std::io::ErrorKind::AddrNotAvailable,
            TraceQueueTelemetryFailureKind::NetworkOffline,
        ),
        (
            std::io::ErrorKind::NetworkDown,
            TraceQueueTelemetryFailureKind::NetworkOffline,
        ),
        (
            std::io::ErrorKind::NetworkUnreachable,
            TraceQueueTelemetryFailureKind::NetworkOffline,
        ),
        (
            std::io::ErrorKind::HostUnreachable,
            TraceQueueTelemetryFailureKind::NetworkOffline,
        ),
        (
            std::io::ErrorKind::ConnectionRefused,
            TraceQueueTelemetryFailureKind::NetworkConnectionRefused,
        ),
    ];

    for (io_kind, expected_kind) in cases {
        let scope = format!(
            "trace-status-sync-provider-io-classification-test-{}",
            Uuid::new_v4()
        );
        write_trace_policy_for_scope(
            Some(&scope),
            &StandingTraceContributionPolicy::default()
                .set_enabled(true)
                .set_ingestion_endpoint("http://127.0.0.1:9/v1/traces".to_string())
                .set_auto_submit_high_value_traces(true)
                .set_min_submission_score(0.0),
        )
        .expect("policy writes");
        write_local_trace_records_for_scope(
            Some(&scope),
            &[submitted_credit_record(1.0, None, None, Vec::new())],
        )
        .expect("local record writes");

        flush_trace_contribution_queue_for_scope_with_credential_provider(
            Some(&scope),
            10,
            &FailingTestUploadCredentialProvider { kind: io_kind },
        )
        .await
        .expect("status sync provider failure is nonfatal during flush");
        let diagnostics = trace_queue_diagnostics_for_scope(Some(&scope)).expect("diagnostics");
        let failure = diagnostics
            .telemetry
            .last_failure
            .as_ref()
            .expect("status sync provider failure recorded");
        assert_eq!(failure.kind, expected_kind);
        assert!(failure.reason.contains("status sync failed"));
        assert!(failure.reason.contains("error_hash="));
        assert!(
            !failure
                .reason
                .contains("credential provider failed while using super-secret-token")
        );
        assert!(!failure.reason.contains("super-secret-token"));

        let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
    }
}
