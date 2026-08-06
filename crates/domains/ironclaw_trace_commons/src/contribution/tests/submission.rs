//! Revoke, policy-aware submit, and upload-claim issuer URL validation and fetching.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use uuid::Uuid;

use crate::contribution::*;

use super::support::*;

#[tokio::test]
async fn revoke_trace_submission_uses_refreshed_upload_claim() {
    let scope = format!("trace-revoke-refresh-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen_for_revoke = seen.clone();
    let app = axum::Router::new().route(
        "/v1/traces/revoke",
        axum::routing::delete(move |headers: axum::http::HeaderMap| {
            let seen = seen_for_revoke.clone();
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
                    axum::http::StatusCode::NO_CONTENT,
                    axum::Json(serde_json::json!({})),
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock trace commons listener binds");
    let endpoint = format!(
        "http://{}/v1/traces/revoke",
        listener.local_addr().expect("local addr")
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: None,
            credit_explanation: Vec::new(),
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    let provider =
        RefreshingTestUploadCredentialProvider::new("stale-upload-claim", "fresh-upload-claim");
    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint(endpoint.clone());
    revoke_trace_submission_for_scope_with_credential_provider(
        Some(&scope),
        submission_id,
        Some(&endpoint),
        &policy,
        &provider,
    )
    .await
    .expect("revoke retries with refreshed claim");

    assert_eq!(
        *seen.lock().expect("seen lock"),
        vec![
            "Bearer stale-upload-claim".to_string(),
            "Bearer fresh-upload-claim".to_string()
        ]
    );
    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert_eq!(records[0].status, NodeTraceSubmissionStatus::Revoked);
    assert!(records[0].revoked_at.is_some());

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn revoke_trace_submission_classifies_http_rejection_through_call_site() {
    let scope = format!("trace-revoke-http-classification-test-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    let app = axum::Router::new().route(
        "/v1/traces/revoke",
        axum::routing::delete(|| async {
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
        "http://{}/v1/traces/revoke",
        listener.local_addr().expect("local addr")
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    write_local_trace_records_for_scope(
        Some(&scope),
        &[NodeTraceSubmissionRecord {
            submission_id,
            trace_id: Uuid::new_v4(),
            endpoint: Some("https://trace.example.com/v1/traces".to_string()),
            status: NodeTraceSubmissionStatus::Submitted,
            server_status: Some("accepted".to_string()),
            submitted_at: Some(Utc::now()),
            revoked_at: None,
            privacy_risk: "low".to_string(),
            redaction_counts: BTreeMap::new(),
            credit_points_pending: 1.0,
            credit_points_final: None,
            credit_explanation: Vec::new(),
            credit_events: Vec::new(),
            history: Vec::new(),
            last_credit_notice_at: None,
            credit_notice_state: TraceCreditNoticeState::default(),
        }],
    )
    .expect("local record writes");

    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_ingestion_endpoint(endpoint.clone());
    let provider =
        RefreshingTestUploadCredentialProvider::new("stale-upload-claim", "fresh-upload-claim");
    let error = revoke_trace_submission_for_scope_with_credential_provider(
        Some(&scope),
        submission_id,
        Some(&endpoint),
        &policy,
        &provider,
    )
    .await
    .expect_err("revoke HTTP rejection should surface to caller");

    assert_eq!(
        trace_queue_telemetry_failure_kind(&error),
        TraceQueueTelemetryFailureKind::HttpRejection
    );
    assert!(!error.to_string().contains("stale-upload-claim"));
    assert!(!error.to_string().contains("fresh-upload-claim"));
    let records = read_local_trace_records_for_scope(Some(&scope)).expect("records read");
    assert_eq!(records[0].status, NodeTraceSubmissionStatus::Submitted);
    assert!(records[0].revoked_at.is_none());

    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn policy_aware_submit_rejects_redirects_without_resending_bearer_token() {
    let token_env = "TRACE_COMMONS_REDIRECT_TEST_TOKEN";
    let _token_guard = EnvVarRestore::set(token_env, "super-secret-token");
    let redirected_authorizations = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let redirected_authorizations_for_handler = redirected_authorizations.clone();
    let app = axum::Router::new()
        .route(
            "/v1/traces",
            axum::routing::post(|| async {
                (
                    axum::http::StatusCode::TEMPORARY_REDIRECT,
                    [(axum::http::header::LOCATION, "/redirected-trace-ingest")],
                )
            }),
        )
        .route(
            "/redirected-trace-ingest",
            axum::routing::post(move |headers: axum::http::HeaderMap| {
                let redirected_authorizations = redirected_authorizations_for_handler.clone();
                async move {
                    let authorization = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("<missing>")
                        .to_string();
                    redirected_authorizations
                        .lock()
                        .expect("redirected authorizations lock")
                        .push(authorization);
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "status": "accepted",
                            "credit_points_pending": 1.0,
                            "explanation": ["accepted after redirect"]
                        })),
                    )
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

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);

    let error = submit_trace_envelope_to_endpoint_with_policy(
        &envelope,
        &endpoint,
        &StandingTraceContributionPolicy::default()
            .set_enabled(true)
            .set_ingestion_endpoint(endpoint.clone())
            .set_bearer_token_env(token_env.to_string()),
    )
    .await
    .expect_err("credentialed trace submission redirects should be rejected");

    assert!(error.to_string().contains("307"));
    assert!(
        redirected_authorizations
            .lock()
            .expect("redirected authorizations lock")
            .is_empty(),
        "bearer token must not be resent to redirected trace endpoints"
    );
}
#[tokio::test]
async fn policy_aware_submit_uses_bounded_request_timeout() {
    let _token_guard = EnvVarRestore::set("TRACE_COMMONS_TEST_TOKEN", "super-secret-token");
    // The 50ms remote-request timeout is supplied via a task-scoped
    // override rather than the process-global
    // `IRONCLAW_TRACE_REMOTE_REQUEST_TIMEOUT_MS` env var, so it cannot leak
    // into other tests' HTTP clients under parallel execution. See the
    // `TEST_REMOTE_REQUEST_TIMEOUT_OVERRIDE` task-local docs.
    // Regression detection is decoupled from a tight wall-clock race: the
    // mock sleeps 10s (>> the 200ms request timeout) and then returns 200
    // OK. A submit that HONORS its bounded timeout returns an
    // `is_timeout()` reqwest error in ~200ms; a submit that IGNORES it
    // sleeps the full 10s and returns the mock's success body, tripping the
    // `Ok(Ok(_))` arm below. The outer 30s watchdog exists ONLY to fail a
    // genuine infinite hang, not to time the request — so it never flakes
    // under an oversubscribed test runtime where reqwest's timer is merely
    // delayed by a few seconds.
    let app = axum::Router::new().route(
        "/v1/traces",
        axum::routing::post(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({
                    "status": "accepted",
                    "credit_points_pending": 1.0,
                    "explanation": ["accepted slowly"]
                })),
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

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);

    let result = TEST_REMOTE_REQUEST_TIMEOUT_OVERRIDE
        .scope(
            Duration::from_millis(200),
            tokio::time::timeout(
                Duration::from_secs(30),
                submit_trace_envelope_to_endpoint_with_policy(
                    &envelope,
                    &endpoint,
                    &StandingTraceContributionPolicy::default()
                        .set_enabled(true)
                        .set_ingestion_endpoint(endpoint.clone())
                        .set_bearer_token_env("TRACE_COMMONS_TEST_TOKEN".to_string()),
                ),
            ),
        )
        .await;
    let error = match result {
        Ok(Err(error)) => error,
        // The submit returned the mock's success body, so it slept the full
        // 10s instead of honoring the 200ms request timeout.
        Ok(Ok(_)) => {
            panic!("slow trace submission should time out via the bounded request timeout")
        }
        // The 30s anti-hang watchdog tripped: the submit neither honored
        // its request timeout nor received the (10s-delayed) response.
        Err(_) => panic!("trace submission hung past the 30s anti-hang watchdog"),
    };

    assert!(
        error.chain().any(|cause| cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_timeout())),
        "unexpected timeout error: {error}"
    );
}
#[tokio::test]
async fn policy_aware_direct_submit_uses_default_credential_provider() {
    let _token_env = EnvVarRestore::set(
        "IRONCLAW_TRACE_COMMONS_DIRECT_SUBMIT_TEST_TOKEN",
        "direct-submit-token",
    );
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen_for_submit = seen.clone();
    let app = axum::Router::new().route(
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
                    seen.lock().expect("seen lock").push(authorization);
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

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    apply_credit_estimate_to_envelope(&mut envelope);
    let policy = StandingTraceContributionPolicy::default()
        .set_bearer_token_env("IRONCLAW_TRACE_COMMONS_DIRECT_SUBMIT_TEST_TOKEN".to_string());

    let receipt = submit_trace_envelope_to_endpoint_with_policy(&envelope, &endpoint, &policy)
        .await
        .expect("direct submit uses policy credentials");

    assert_eq!(receipt.status, "accepted");
    assert_eq!(
        *seen.lock().expect("seen lock"),
        vec!["Bearer direct-submit-token".to_string()]
    );
}
/// Serves exactly one request at the returned `/v1/traces` endpoint: drains
/// it, answers with `status_line` plus a `content-length: 4096` head but only
/// the first bytes of the body, then closes — so the response HEAD completes
/// (`send()` succeeds) while `.text()`'s body stream dies mid-read.
async fn spawn_truncated_body_trace_mock(status_line: &'static str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock trace commons listener binds");
    let endpoint = format!(
        "http://{}/v1/traces",
        listener.local_addr().expect("local addr")
    );
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut socket, _) = listener.accept().await.expect("accept");
        // Drain the request: headers, then the Content-Length'd JSON body.
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.expect("read request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(headers_end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..headers_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().expect("content length"))
                    })
                    .unwrap_or(0);
                if request.len() >= headers_end + 4 + content_length {
                    break;
                }
            }
        }
        // The head completes with a Content-Length the body never reaches,
        // then the socket closes: `send()` succeeds, the body read fails.
        socket
            .write_all(
                format!(
                    "{status_line}\r\ncontent-type: application/json\r\ncontent-length: 4096\r\n\r\n{{\"status\":"
                )
                .as_bytes(),
            )
            .await
            .expect("write truncated response");
        socket.flush().await.expect("flush");
        drop(socket);
    });
    endpoint
}
/// A 2xx whose body STREAM fails mid-read is a transport failure, not a
/// protocol one: the old `response.text().await.unwrap_or_default()` collapsed
/// the read error into an empty body, which the strict receipt parse (#7144)
/// then reported as `response_invalid` ("server returned success with a body
/// that is not a submission receipt") — blaming the server's payload for the
/// client's dropped connection and recording the wrong telemetry kind.
#[tokio::test]
async fn submit_preserves_response_body_read_failure_as_request_failure() {
    let endpoint = spawn_truncated_body_trace_mock("HTTP/1.1 200 OK").await;

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    let failure = submit_trace_envelope_to_endpoint_with_token(&envelope, &endpoint, "test-token")
        .await
        .expect_err("a failed body read must surface as an error");

    assert_ne!(
        failure.kind,
        TraceQueueTelemetryFailureKind::Submission,
        "a dropped body stream is a transport failure and must not be \
         classified as a server-protocol (`response_invalid`) failure: {failure}"
    );
    assert!(
        failure
            .to_string()
            .contains("trace submission response body"),
        "the failure must name the body read as the failing operation, got: {failure}"
    );
}
/// A non-2xx whose REJECTION body read fails must keep both truths: the
/// received HTTP status (it classifies the rejection — the 401/403 auth-retry
/// and the Credential/HttpRejection telemetry split both key off it, so the
/// failure must stay an `http_rejection`) and the body read's own cause. The
/// old path collapsed the read error with `.unwrap_or_default()`, reporting
/// "rejected by 503" with an empty detail as though the server had sent an
/// empty body — hiding that the transport died while the rejection detail was
/// being read (`.claude/rules/error-handling.md` bans exactly that collapse).
#[tokio::test]
async fn submit_preserves_rejection_body_read_failure_cause_with_status() {
    let endpoint = spawn_truncated_body_trace_mock("HTTP/1.1 503 Service Unavailable").await;

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    let failure = submit_trace_envelope_to_endpoint_with_token(&envelope, &endpoint, "test-token")
        .await
        .expect_err("a rejected submission must surface as an error");

    assert_eq!(
        failure.kind,
        TraceQueueTelemetryFailureKind::HttpRejection,
        "a rejection whose body read fails is still an HTTP rejection: {failure}"
    );
    assert!(
        failure.to_string().contains("503"),
        "the rejection must keep the received HTTP status, got: {failure}"
    );
    assert!(
        failure.to_string().contains("rejection body read failed"),
        "the rejection must retain the body-read failure's cause instead of \
         degrading to an empty detail, got: {failure}"
    );
}
/// A 200 whose body is `{}` — or any JSON object with no server-sent `status`
/// — is not a submission acknowledgement. Every real Trace Commons ingest
/// response (and every fixture in this workspace) names what happened to the
/// submission (`{"status": "accepted", ...}`), and callers persist that word
/// unconditionally as `server_status` truth. The receipt's serde default used
/// to fabricate `status: "submitted"` for a body that never said so, so a
/// proxy or faulty server answering `200 {}` counted as submitted — and the
/// flush caller then deleted the queued envelope, destroying the only
/// retryable copy (the #7144 failure class, resurfacing through the wire
/// type's defaults instead of a synthesizing code branch).
#[tokio::test]
async fn submit_rejects_success_response_without_explicit_server_status() {
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits_for_mock = hits.clone();
    let app = axum::Router::new().route(
        "/v1/traces",
        axum::routing::post(move || {
            let hits = hits_for_mock.clone();
            async move {
                if hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    // A proxy's empty-object success.
                    axum::Json(serde_json::json!({}))
                } else {
                    // A faulty server's status-less non-empty object.
                    axum::Json(serde_json::json!({ "credit_points_pending": 1.0 }))
                }
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

    let raw = RawTraceContribution::from_recorded_trace(
        &sample_trace(),
        RecordedTraceContributionOptions::default(),
    );
    let envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");

    for case in ["empty object", "status-less object"] {
        let failure =
            submit_trace_envelope_to_endpoint_with_token(&envelope, &endpoint, "test-token")
                .await
                .expect_err(
                    "a 200 whose body never states a server status must not count as submitted",
                );
        assert_eq!(
            failure.kind,
            TraceQueueTelemetryFailureKind::Submission,
            "a status-less 200 body ({case}) is a protocol violation, not a transport one: {failure}"
        );
        assert!(
            failure.to_string().contains("not a submission receipt"),
            "the failure ({case}) must report the body as not being a receipt, got: {failure}"
        );
    }
}
#[test]
fn upload_claim_issuer_url_validation_rejects_unsafe_targets() {
    let allowed_hosts = BTreeSet::from(["issuer.example.com".to_string()]);
    assert!(
        validate_trace_upload_claim_issuer_url(
            &reqwest::Url::parse("https://issuer.example.com/v1/claims").expect("url"),
            &allowed_hosts,
        )
        .is_ok()
    );

    for unsafe_url in [
        "http://issuer.example.com/v1/claims",
        "https://user:secret@issuer.example.com/v1/claims",
        "https://issuer.example.com/v1/claims?token=secret",
        "https://issuer.example.com/v1/claims#fragment",
        "https://metadata.google.internal/v1/claims",
    ] {
        assert!(
            validate_trace_upload_claim_issuer_url(
                &reqwest::Url::parse(unsafe_url).expect("url"),
                &allowed_hosts,
            )
            .is_err(),
            "{unsafe_url} should be rejected"
        );
    }
}
#[test]
fn upload_claim_issuer_url_validation_allows_literal_loopback_dev() {
    // The loopback-HTTP dev invite form writes a loopback claim endpoint
    // into the policy; the validator must accept the same exception or a
    // successful loopback onboarding can never mint a claim.
    for (url, host) in [
        ("http://127.0.0.1:3917/v1/trace-upload-claim", "127.0.0.1"),
        ("http://localhost:3917/v1/trace-upload-claim", "localhost"),
        ("http://[::1]:3917/v1/trace-upload-claim", "[::1]"),
        ("https://127.0.0.1/v1/trace-upload-claim", "127.0.0.1"),
    ] {
        let allowed_hosts = BTreeSet::from([host.to_string()]);
        assert!(
            validate_trace_upload_claim_issuer_url(
                &reqwest::Url::parse(url).expect("url"),
                &allowed_hosts,
            )
            .is_ok(),
            "{url} should be accepted under the loopback dev exception"
        );
    }

    // The exception is literal loopback only: plain-HTTP private/internal
    // hosts and loopback-suffixed hostnames stay rejected, and loopback
    // still has to pass the allowlist.
    let allowed = BTreeSet::from(["10.0.0.5".to_string(), "foo.localhost".to_string()]);
    for unsafe_url in [
        "http://10.0.0.5/v1/trace-upload-claim",
        "http://192.168.1.10/v1/trace-upload-claim",
        "http://foo.localhost/v1/trace-upload-claim",
        "https://foo.localhost/v1/trace-upload-claim",
    ] {
        assert!(
            validate_trace_upload_claim_issuer_url(
                &reqwest::Url::parse(unsafe_url).expect("url"),
                &allowed,
            )
            .is_err(),
            "{unsafe_url} should be rejected"
        );
    }
    assert!(
        validate_trace_upload_claim_issuer_url(
            &reqwest::Url::parse("http://127.0.0.1:3917/v1/trace-upload-claim").expect("url"),
            &BTreeSet::from(["issuer.example.com".to_string()]),
        )
        .is_err(),
        "loopback host not on the allowlist should be rejected"
    );
}
#[test]
fn ingest_url_validation_allows_literal_loopback_dev() {
    assert!(
        validate_trace_commons_ingest_url(
            &reqwest::Url::parse("http://127.0.0.1:3917/v1/traces").expect("url")
        )
        .is_ok()
    );
    assert!(
        validate_trace_commons_ingest_url(
            &reqwest::Url::parse("http://10.0.0.5/v1/traces").expect("url")
        )
        .is_err()
    );
    assert!(
        validate_trace_commons_ingest_url(
            &reqwest::Url::parse("https://ingest.example.com/v1/traces").expect("url")
        )
        .is_ok()
    );
}
#[tokio::test]
async fn fetch_trace_upload_claim_from_issuer_accepts_loopback_dev_issuer() {
    // Regression: a loopback-HTTP dev onboarding writes
    // `http://127.0.0.1:<port>/v1/trace-upload-claim` into the policy. The
    // real claim fetch must honor the same loopback exception end-to-end
    // (URL validator + pinned DNS resolution), or a successfully onboarded
    // loopback enrollment can never mint a claim.
    let token = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "dev-key-1"}));
    let claim_token = token.clone();
    let app = axum::Router::new().route(
        "/v1/trace-upload-claim",
        axum::routing::post(move || {
            let token = claim_token.clone();
            async move {
                axum::Json(serde_json::json!({
                    "access_token": token,
                    "token_type": "Bearer",
                    "expires_in": 300,
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock issuer listener binds");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let scope_dir = tempfile::tempdir().expect("tempdir");
    crate::onboarding::DeviceKeypair::load_or_generate_pending(
        scope_dir.path(),
        "loopback-invite-hash",
    )
    .expect("generate pending device key")
    .promote(scope_dir.path(), "tenant-dev")
    .expect("promote device key");

    let policy = StandingTraceContributionPolicy::default()
        .set_enabled(true)
        .set_auth_mode(TraceUploadAuthMode::DeviceKey)
        .set_upload_token_issuer_url(format!("http://{addr}/v1/trace-upload-claim"))
        .set_upload_token_issuer_allowed_hosts(BTreeSet::from(["127.0.0.1".to_string()]))
        .set_upload_token_tenant_id("tenant-dev".to_string())
        .set_upload_token_audience("trace-commons".to_string());
    let context = TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: Vec::new(),
        scope_dir: Some(scope_dir.path().to_path_buf()),
        subject: None,
    };
    let claim = fetch_trace_upload_claim_from_issuer(&policy, &context, None)
        .await
        .expect("loopback dev issuer mints a claim");
    assert_eq!(claim.access_token, token);
}
#[tokio::test]
async fn fetch_claim_sends_subject_when_present() {
    use std::sync::{Arc, Mutex};
    let captured: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = captured.clone();
    let token = test_jwt_with_header(serde_json::json!({"alg":"EdDSA","kid":"dev-key-1"}));
    let claim_token = token.clone();
    let app = axum::Router::new().route(
        "/v1/trace-upload-claim",
        axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let cap = cap.clone();
            let token = claim_token.clone();
            async move {
                cap.lock().unwrap().push(body);
                axum::Json(serde_json::json!({
                    "access_token": token, "token_type": "Bearer", "expires_in": 300
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let scope_dir = tempfile::tempdir().unwrap();
    crate::onboarding::DeviceKeypair::load_or_generate_pending(scope_dir.path(), "h")
        .unwrap()
        .promote(scope_dir.path(), "tenant-dev")
        .unwrap();

    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
        upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
            "127.0.0.1".to_string()
        ]),
        upload_token_tenant_id: Some("tenant-dev".to_string()),
        upload_token_audience: Some("trace-commons".to_string()),
        ..Default::default()
    };
    let context = TraceUploadClaimContext {
        trace_id: None,
        submission_id: None,
        consent_scopes: vec![ConsentScope::DebuggingEvaluation],
        allowed_uses: Vec::new(),
        scope_dir: Some(scope_dir.path().to_path_buf()),
        subject: Some("sha256:alice".to_string()),
    };
    let _ = fetch_trace_upload_claim_from_issuer(&policy, &context, None)
        .await
        .unwrap();

    let bodies = captured.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["subject"], "sha256:alice");
}
#[test]
fn upload_claim_response_requires_eddsa_jwt_with_kid() {
    let token = test_jwt_with_header(serde_json::json!({
        "alg": "EdDSA",
        "kid": "managed-key-1"
    }));
    validate_trace_upload_claim_response(&TraceUploadClaimIssuerResponse {
        access_token: token,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        expires_in: Some(300),
    })
    .expect("EdDSA token with kid is accepted for client-side transport");

    let non_eddsa = test_jwt_with_header(serde_json::json!({
        "alg": "HS256",
        "kid": "managed-key-1"
    }));
    let error = validate_trace_upload_claim_response(&TraceUploadClaimIssuerResponse {
        access_token: non_eddsa,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        expires_in: Some(300),
    })
    .expect_err("non-EdDSA upload claims are rejected");
    assert!(error.to_string().contains("EdDSA"));

    let missing_kid = test_jwt_with_header(serde_json::json!({
        "alg": "EdDSA"
    }));
    let error = validate_trace_upload_claim_response(&TraceUploadClaimIssuerResponse {
        access_token: missing_kid,
        token_type: Some("Bearer".to_string()),
        expires_at: None,
        expires_in: Some(300),
    })
    .expect_err("managed upload claims require kid");
    assert!(error.to_string().contains("kid"));
}
