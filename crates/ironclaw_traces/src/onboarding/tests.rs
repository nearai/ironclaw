use super::*;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{Router, extract::State, routing::post};

struct MockIssuer {
    pub addr: SocketAddr,
    pub received: Arc<Mutex<Vec<serde_json::Value>>>,
}

/// Axum handler state for the mock issuer:
/// (canned response JSON, status code, recorded request bodies).
type MockState = Arc<(
    serde_json::Value,
    axum::http::StatusCode,
    Arc<Mutex<Vec<serde_json::Value>>>,
)>;

/// Sentinel `device_key_id` value: the mock replaces it with the id derived
/// from the request's submitted public key (see handler).
const ECHO_DEVICE_KEY_ID: &str = "ECHO_DEVICE_KEY_ID";

/// Derive `sha256:<hex>` of the base64-standard-decoded public key bytes —
/// mirrors `device_key::device_key_id_from_pubkey` (the server's scheme).
fn derive_device_key_id(pubkey_b64: &str) -> Option<String> {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(pubkey_b64)
        .ok()?;
    Some(format!("sha256:{}", hex::encode(Sha256::digest(&bytes))))
}

/// Spawn a mock `/v1/onboard` server on 127.0.0.1:0.
/// `make_response` receives the bound address so it can embed the correct
/// `issuer_url` origin in the response JSON.
async fn spawn_mock_issuer<F>(make_response: F, status: axum::http::StatusCode) -> MockIssuer
where
    F: Fn(SocketAddr) -> serde_json::Value + Send + Sync + 'static,
{
    let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));

    // Bind first so we know the addr before building the response JSON.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock issuer binds");
    let addr = listener.local_addr().expect("mock issuer local addr");

    // Build the response JSON eagerly now that we know the addr.
    let response_body = make_response(addr);

    // State shared with the handler: (response_json, status_code, received_log).
    let state: MockState = Arc::new((response_body, status, Arc::clone(&received)));

    async fn handler(
        State(state): State<MockState>,
        axum::Json(body): axum::Json<serde_json::Value>,
    ) -> axum::response::Response {
        let mut response = state.0.clone();
        // Realistic server behavior: derive device_key_id from the
        // contributor's submitted public key. If the canned response uses
        // the `ECHO_DEVICE_KEY_ID` sentinel, replace it with the value
        // derived from this request so the client's local cross-check
        // passes. Tests that want a *mismatch* set an explicit value.
        if response.get("device_key_id").and_then(|v| v.as_str()) == Some(ECHO_DEVICE_KEY_ID)
            && let Some(pubkey_b64) = body.get("device_public_key").and_then(|v| v.as_str())
            && let Some(derived) = derive_device_key_id(pubkey_b64)
        {
            response["device_key_id"] = serde_json::Value::String(derived);
        }
        state.2.lock().unwrap().push(body);
        let json_bytes = serde_json::to_vec(&response).unwrap();
        axum::response::Response::builder()
            .status(state.1)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(json_bytes))
            .unwrap()
    }

    let app = Router::new()
        .route("/v1/onboard", post(handler))
        .with_state(state);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    MockIssuer { addr, received }
}

/// Stateful mock issuer on a SINGLE origin: the first `fail_first` requests
/// return HTTP 500, every request after that returns `make_response(addr)`
/// with 200. Used by retry tests so a retry hits the SAME origin (and thus
/// the same origin-scoped pending-key file) instead of a second mock on a
/// different port — which would (correctly) stage a fresh key under the
/// per-issuer pending-key scoping.
async fn spawn_flaky_mock_issuer<F>(fail_first: usize, make_response: F) -> MockIssuer
where
    F: Fn(SocketAddr) -> serde_json::Value + Send + Sync + 'static,
{
    use std::sync::atomic::{AtomicUsize, Ordering};

    let received: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock issuer binds");
    let addr = listener.local_addr().expect("mock issuer local addr");
    let response_body = make_response(addr);

    type FlakyState = Arc<(
        serde_json::Value,
        usize,
        AtomicUsize,
        Arc<Mutex<Vec<serde_json::Value>>>,
    )>;
    let state: FlakyState = Arc::new((
        response_body,
        fail_first,
        AtomicUsize::new(0),
        Arc::clone(&received),
    ));

    async fn handler(
        State(state): State<FlakyState>,
        axum::Json(body): axum::Json<serde_json::Value>,
    ) -> axum::response::Response {
        let n = state.2.fetch_add(1, Ordering::SeqCst);
        if n < state.1 {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "application/json")
                .body(axum::body::Body::from("\"error\""))
                .unwrap();
        }
        let mut response = state.0.clone();
        if response.get("device_key_id").and_then(|v| v.as_str()) == Some(ECHO_DEVICE_KEY_ID)
            && let Some(pubkey_b64) = body.get("device_public_key").and_then(|v| v.as_str())
            && let Some(derived) = derive_device_key_id(pubkey_b64)
        {
            response["device_key_id"] = serde_json::Value::String(derived);
        }
        state.3.lock().unwrap().push(body);
        let json_bytes = serde_json::to_vec(&response).unwrap();
        axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(json_bytes))
            .unwrap()
    }

    let app = Router::new()
        .route("/v1/onboard", post(handler))
        .with_state(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    MockIssuer { addr, received }
}

fn ok_response(addr: SocketAddr, ingest_url: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "trace_commons.onboard_response.v1",
        "tenant_id": "tenant-a",
        "ingest_url": ingest_url,
        "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
        "audience": "trace-commons-ingest",
        "device_key_id": ECHO_DEVICE_KEY_ID,
    })
}

#[tokio::test]
async fn successful_onboard_writes_policy_and_promotes_key() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |addr| ok_response(addr, "https://ingest.example.com"),
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST01", mock.addr.port());
    let consents = OnboardConsents {
        include_message_text: true,
        include_tool_payloads: false,
    };

    let outcome = onboard_at_dir(dir.path(), &invite_url, consents)
        .await
        .expect("onboard succeeds");

    // Outcome fields.
    assert_eq!(outcome.tenant_id, "tenant-a");
    assert_eq!(outcome.ingest_url, "https://ingest.example.com");
    assert_eq!(
        outcome.issuer_url,
        format!("http://127.0.0.1:{}", mock.addr.port())
    );

    // Policy file exists with correct fields.
    let policy_path = dir.path().join("policy.json");
    assert!(policy_path.exists(), "policy.json must be written");
    let policy: StandingTraceContributionPolicy =
        serde_json::from_str(&std::fs::read_to_string(&policy_path).unwrap()).unwrap();
    assert!(policy.enabled);
    assert_eq!(policy.auth_mode, TraceUploadAuthMode::DeviceKey);
    // upload_token_issuer_url must be the full claim endpoint, not just the origin.
    let expected_issuer_url = format!(
        "http://127.0.0.1:{}/v1/trace-upload-claim",
        mock.addr.port()
    );
    assert_eq!(
        policy.upload_token_issuer_url.as_deref(),
        Some(expected_issuer_url.as_str()),
        "upload_token_issuer_url must be the full claim endpoint (origin + /v1/trace-upload-claim)"
    );
    // Verify parsed path is /v1/trace-upload-claim and host matches invite.
    let issuer_parsed = reqwest::Url::parse(expected_issuer_url.as_str()).unwrap();
    assert_eq!(issuer_parsed.path(), "/v1/trace-upload-claim");
    assert_eq!(issuer_parsed.host_str().unwrap(), format!("127.0.0.1"));
    assert_eq!(
        policy.ingestion_endpoint.as_deref(),
        Some("https://ingest.example.com")
    );
    assert!(policy.include_message_text);
    assert!(!policy.include_tool_payloads);
    assert_eq!(
        policy.device_key_id.as_deref(),
        Some(outcome.device_key_id.as_str())
    );

    // Tenant key file exists; pending file gone.
    let invite = ParsedInvite::parse(&invite_url).unwrap();
    let tenant_key = dir.path().join(format!(
        "device_keys/{}.json",
        device_key::tenant_hash("tenant-a")
    ));
    assert!(tenant_key.exists(), "tenant key file must exist");
    let pending = dir.path().join(format!(
        "device_keys/pending/{}.json",
        invite.pending_key_hash()
    ));
    assert!(
        !pending.exists(),
        "pending file must be gone after the post-policy-write finalize"
    );

    // Exactly 1 request received with a non-empty device_public_key.
    let received = mock.received.lock().unwrap();
    assert_eq!(received.len(), 1);
    let dkpub = received[0]["device_public_key"].as_str().unwrap();
    assert!(!dkpub.is_empty());
}

#[tokio::test]
async fn issuer_url_mismatch_rejects_onboard() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |_addr| {
            serde_json::json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-a",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": "https://evil.example.com",
                "audience": "trace-commons-ingest",
                "device_key_id": "sha256:x",
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST02", mock.addr.port());

    let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("mismatch must be rejected");

    assert!(
        matches!(err, OnboardError::IssuerOriginMismatch { .. }),
        "expected IssuerOriginMismatch, got: {err}"
    );
    assert!(
        !dir.path().join("policy.json").exists(),
        "policy.json must not be written on mismatch"
    );
}

#[tokio::test]
async fn invite_not_valid_discards_pending_key() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |_addr| serde_json::json!({ "error": "InviteNotValid" }),
        axum::http::StatusCode::FORBIDDEN,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST03", mock.addr.port());

    let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("terminal rejection must fail");

    assert!(
        matches!(
            err,
            OnboardError::InviteRejected(OnboardErrorCode::InviteNotValid)
        ),
        "expected InviteRejected(InviteNotValid), got: {err}"
    );

    // Pending key directory should be empty or not contain the pending file.
    let invite = ParsedInvite::parse(&invite_url).unwrap();
    let pending = dir.path().join(format!(
        "device_keys/pending/{}.json",
        invite.pending_key_hash()
    ));
    assert!(
        !pending.exists(),
        "pending key must be discarded on InviteNotValid"
    );
}

#[tokio::test]
async fn transient_server_error_keeps_pending_key() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |_addr| serde_json::json!("garbage body that is not a valid error"),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST04", mock.addr.port());

    let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("transient error must fail");

    assert!(
        matches!(err, OnboardError::Network { .. }),
        "expected Network error, got: {err}"
    );

    // Pending file must still exist for retry.
    let invite = ParsedInvite::parse(&invite_url).unwrap();
    let pending = dir.path().join(format!(
        "device_keys/pending/{}.json",
        invite.pending_key_hash()
    ));
    assert!(
        pending.exists(),
        "pending key must be retained on transient error"
    );
}

#[tokio::test]
async fn non_https_ingest_url_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |addr| {
            serde_json::json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-a",
                "ingest_url": "http://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST05", mock.addr.port());

    let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("non-https ingest_url must be rejected");

    assert!(
        matches!(err, OnboardError::InsecureIngestUrl { .. }),
        "expected InsecureIngestUrl, got: {err}"
    );
}

#[test]
fn ingest_url_with_embedded_credentials_is_rejected() {
    // A server-controlled onboarding response must not be able to smuggle
    // raw credentials into policy.json via the ingest_url userinfo.
    for url in [
        "https://user:pass@ingest.example.com/v1/traces",
        "https://user@ingest.example.com/v1/traces",
    ] {
        assert!(
            matches!(
                ensure_https_or_loopback_url(url),
                Err(OnboardError::InsecureIngestUrl { .. })
            ),
            "{url} (embedded credentials) must be rejected"
        );
    }
    // The same host without userinfo is still accepted.
    assert!(ensure_https_or_loopback_url("https://ingest.example.com/v1/traces").is_ok());
}

/// Loopback http ingest_url is allowed (same rule as invite parsing).
#[tokio::test]
async fn loopback_ingest_url_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |addr| {
            serde_json::json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-b",
                "ingest_url": "http://127.0.0.1:9999",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST05B", mock.addr.port());

    let outcome = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect("loopback ingest_url must be accepted");
    assert_eq!(outcome.ingest_url, "http://127.0.0.1:9999");
}

#[tokio::test]
async fn community_urls_pass_through_when_https_and_drop_when_not() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |addr| {
            serde_json::json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-a",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
                "profile_url": "https://tracecommons.ai/profile",
                "leaderboard_url": "http://insecure.example.com/lb",
                "community_url": "https://tracecommons.ai",
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST06", mock.addr.port());

    let outcome = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect("onboard succeeds");

    assert_eq!(
        outcome.profile_url.as_deref(),
        Some("https://tracecommons.ai/profile"),
        "HTTPS profile_url must pass through"
    );
    assert!(
        outcome.leaderboard_url.is_none(),
        "non-HTTPS leaderboard_url must be dropped"
    );
    assert_eq!(
        outcome.community_url.as_deref(),
        Some("https://tracecommons.ai"),
        "HTTPS community_url must pass through"
    );
}

#[tokio::test]
async fn retry_after_transient_failure_reuses_same_keypair() {
    let dir = tempfile::tempdir().unwrap();

    // One issuer origin that fails the first request (500 → Network error)
    // then succeeds. The retry MUST hit the same origin so it reuses the
    // origin-scoped staged key; a second mock on a different port would
    // correctly stage a fresh key under per-issuer pending-key scoping.
    let mock =
        spawn_flaky_mock_issuer(1, |addr| ok_response(addr, "https://ingest.example.com")).await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST07", mock.addr.port());

    let _ = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("first call must fail");

    // Read the staged pending key's public key for later comparison.
    let invite = ParsedInvite::parse(&invite_url).unwrap();
    let pending_path = dir.path().join(format!(
        "device_keys/pending/{}.json",
        invite.pending_key_hash()
    ));
    assert!(
        pending_path.exists(),
        "pending file must exist after transient failure"
    );
    let pending_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&pending_path).unwrap()).unwrap();
    let first_pubkey = pending_json["public_key"].as_str().unwrap().to_owned();

    // Retry against the SAME origin → second request succeeds and reuses
    // the staged keypair.
    let outcome = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect("second call succeeds");

    let received = mock.received.lock().unwrap();
    // Only the successful (second) request is recorded by the flaky mock.
    assert_eq!(received.len(), 1);
    let second_pubkey = received[0]["device_public_key"].as_str().unwrap();
    assert_eq!(
        first_pubkey, second_pubkey,
        "retry must reuse the staged keypair (same public key)"
    );
    assert!(!outcome.device_key_id.is_empty());
}

#[tokio::test]
async fn policy_write_failure_keeps_pending_and_retry_reuses_key() {
    let dir = tempfile::tempdir().unwrap();

    // Force the policy write to fail by pre-creating <dir>/policy.json as a
    // DIRECTORY: `write_json_file` renames a temp file onto that path, which
    // fails deterministically when the destination is a non-empty dir.
    let policy_path = dir.path().join("policy.json");
    std::fs::create_dir(&policy_path).unwrap();
    std::fs::write(policy_path.join("blocker"), b"x").unwrap();

    let mock = spawn_mock_issuer(
        |addr| ok_response(addr, "https://ingest.example.com"),
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST08", mock.addr.port());

    let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("policy write must fail");
    assert!(
        matches!(err, OnboardError::Persist { .. }),
        "expected Persist, got: {err}"
    );

    // The pending key MUST survive the failed policy write (no lockout).
    let invite = ParsedInvite::parse(&invite_url).unwrap();
    let pending_path = dir.path().join(format!(
        "device_keys/pending/{}.json",
        invite.pending_key_hash()
    ));
    assert!(
        pending_path.exists(),
        "pending key must survive a failed policy write"
    );
    let staged: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&pending_path).unwrap()).unwrap();
    let staged_key_id = staged["device_key_id"].as_str().unwrap().to_owned();

    // Clear the blocker so the retry's policy write can succeed.
    std::fs::remove_dir_all(dir.path().join("policy.json")).unwrap();

    // Retry against the SAME origin (the original mock still serves OK) so
    // the origin-scoped staged key is reused → success with the SAME
    // device_key_id (server idempotency + reused pending key). A second mock
    // on a different port would correctly stage a fresh per-issuer key.
    let outcome = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect("retry succeeds after the blocker is cleared");

    assert_eq!(
        outcome.device_key_id, staged_key_id,
        "retry must reuse the same device key"
    );
    assert!(dir.path().join("policy.json").is_file());
    assert!(
        !pending_path.exists(),
        "pending key removed after the successful retry finalize"
    );
}

#[tokio::test]
async fn wrong_schema_version_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |addr| {
            serde_json::json!({
                "schema_version": "trace_commons.onboard_response.v2",
                "tenant_id": "tenant-a",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST09", mock.addr.port());

    let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("unexpected schema_version must be rejected");
    assert!(
        matches!(err, OnboardError::MalformedResponse { .. }),
        "expected MalformedResponse, got: {err}"
    );
    assert!(!dir.path().join("policy.json").exists());
}

/// The policy's upload_token_issuer_url must be the full claim endpoint
/// (origin + /v1/trace-upload-claim), not the bare origin.  A bare-origin
/// value would cause fetch_trace_upload_claim_from_issuer to POST to the
/// root instead of the claim endpoint.
#[tokio::test]
async fn policy_upload_token_issuer_url_is_claim_endpoint_not_origin() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |addr| ok_response(addr, "https://ingest.example.com"),
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST11", mock.addr.port());

    onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect("onboard succeeds");

    let policy: StandingTraceContributionPolicy =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("policy.json")).unwrap())
            .unwrap();

    let issuer_url = policy
        .upload_token_issuer_url
        .as_deref()
        .expect("upload_token_issuer_url must be set");

    // Must NOT be the bare origin.
    assert_ne!(
        issuer_url,
        format!("http://127.0.0.1:{}", mock.addr.port()).as_str(),
        "upload_token_issuer_url must not be the bare origin"
    );

    // Must be the full claim endpoint.
    assert_eq!(
        issuer_url,
        format!(
            "http://127.0.0.1:{}/v1/trace-upload-claim",
            mock.addr.port()
        )
        .as_str(),
        "upload_token_issuer_url must be origin + /v1/trace-upload-claim"
    );

    // Parse and assert path and host separately.
    let parsed = reqwest::Url::parse(issuer_url).expect("issuer_url must be parseable");
    assert_eq!(
        parsed.path(),
        "/v1/trace-upload-claim",
        "parsed path must be /v1/trace-upload-claim"
    );
    assert_eq!(
        parsed.host_str().unwrap(),
        "127.0.0.1",
        "host must match invite host"
    );
    // Note: a full end-to-end claim-fetch-path test asserting the POST hits
    // /v1/trace-upload-claim would require a mock claim endpoint; deferred
    // because the policy URL path assertion above captures the same invariant
    // (fetch_trace_upload_claim_from_issuer POSTs to parsed.clone() as-is).
}

/// Verify that a terminal invite rejection returns InviteRejected (the
/// primary error), not a DeviceKey error, even if discard_pending fails.
/// This is the normal path — discard should succeed, and the primary error
/// must be returned.
#[tokio::test]
async fn terminal_rejection_returns_invite_rejected_as_primary_error() {
    let dir = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |_addr| serde_json::json!({ "error": "InviteMalformed" }),
        axum::http::StatusCode::FORBIDDEN,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST12", mock.addr.port());

    let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("terminal rejection must fail");

    // The error must be InviteRejected (the primary), not DeviceKey.
    assert!(
        matches!(
            err,
            OnboardError::InviteRejected(OnboardErrorCode::InviteMalformed)
        ),
        "expected InviteRejected(InviteMalformed), got: {err}"
    );
}

#[tokio::test]
async fn mismatched_device_key_id_rejected() {
    let dir = tempfile::tempdir().unwrap();
    // Explicit (non-sentinel) device_key_id that cannot match the locally
    // derived value → the cross-check must reject it as a tamper signal.
    let mock = spawn_mock_issuer(
        |addr| {
            serde_json::json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-a",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVTEST10", mock.addr.port());

    let err = onboard_at_dir(dir.path(), &invite_url, OnboardConsents::default())
        .await
        .expect_err("mismatched device_key_id must be rejected");
    assert!(
        matches!(err, OnboardError::MalformedResponse { .. }),
        "expected MalformedResponse, got: {err}"
    );
    assert!(!dir.path().join("policy.json").exists());
}

// ── Fake-sink tests (no network) ──────────────────────────────────────────

/// A no-network sink that returns a canned status + body and records the
/// posted URL — proves the seam works without reqwest/loopback.
struct FakeSink {
    status: u16,
    body: Vec<u8>,
    posted_url: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl OnboardingHttpSink for FakeSink {
    async fn post_onboard(
        &self,
        url: &str,
        _body: Vec<u8>,
    ) -> Result<OnboardHttpResponse, OnboardError> {
        *self.posted_url.lock().unwrap() = Some(url.to_string());
        Ok(OnboardHttpResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
}

/// Build the canned 200 onboard response body that the client will accept,
/// echoing the device_key_id derived from the request's pending public key.
fn canned_ok_body(dir: &Path, invite_url: &str) -> Vec<u8> {
    // Stage the pending key the same way onboard_at_dir will, so we can
    // derive the device_key_id the client will cross-check against.
    let invite = ParsedInvite::parse(invite_url).unwrap();
    let pending = DeviceKeypair::load_or_generate_pending(dir, &invite.pending_key_hash()).unwrap();
    let device_key_id = derive_device_key_id(&pending.public_key_b64).unwrap();
    serde_json::to_vec(&serde_json::json!({
        "schema_version": "trace_commons.onboard_response.v1",
        "tenant_id": "tenant-fake",
        "ingest_url": "https://ingest.example.com",
        "issuer_url": invite.origin,
        "audience": "trace-commons-ingest",
        "device_key_id": device_key_id,
    }))
    .unwrap()
}

#[tokio::test]
async fn fake_sink_200_writes_policy_through_seam() {
    let dir = tempfile::tempdir().unwrap();
    // Use a loopback-shaped invite URL so origin anchoring passes; no server
    // is bound — the fake sink never touches the network.
    let invite_url = "http://127.0.0.1:7/onboard#INVFAKE01";
    let body = canned_ok_body(dir.path(), invite_url);
    let posted_url = Arc::new(Mutex::new(None));
    let sink = FakeSink {
        status: 200,
        body,
        posted_url: Arc::clone(&posted_url),
    };

    let outcome =
        onboard_at_dir_with_sink(dir.path(), invite_url, OnboardConsents::default(), &sink)
            .await
            .expect("fake-sink onboard succeeds");

    assert_eq!(outcome.tenant_id, "tenant-fake");
    assert!(
        dir.path().join("policy.json").exists(),
        "policy.json must be written through the sink seam"
    );
    assert_eq!(
        posted_url.lock().unwrap().as_deref(),
        Some("http://127.0.0.1:7/v1/onboard"),
        "sink must be posted the onboard endpoint"
    );
}

#[tokio::test]
async fn fake_sink_403_invite_not_valid_discards_pending() {
    let dir = tempfile::tempdir().unwrap();
    let invite_url = "http://127.0.0.1:7/onboard#INVFAKE02";
    let sink = FakeSink {
        status: 403,
        body: serde_json::to_vec(&serde_json::json!({ "error": "InviteNotValid" })).unwrap(),
        posted_url: Arc::new(Mutex::new(None)),
    };

    let err = onboard_at_dir_with_sink(dir.path(), invite_url, OnboardConsents::default(), &sink)
        .await
        .expect_err("4xx invite rejection must fail through the sink");

    assert!(
        matches!(
            err,
            OnboardError::InviteRejected(OnboardErrorCode::InviteNotValid)
        ),
        "expected InviteRejected(InviteNotValid), got: {err}"
    );
    let invite = ParsedInvite::parse(invite_url).unwrap();
    let pending = dir.path().join(format!(
        "device_keys/pending/{}.json",
        invite.pending_key_hash()
    ));
    assert!(
        !pending.exists(),
        "pending key must be discarded on InviteNotValid through the sink"
    );
}

/// Verify that the instance-level onboarding path writes the policy to the
/// scope-None location (no `users/<hash>` segment) under an isolated base.
///
/// Uses `onboard_at_dir_with_sink` targeting `tempdir/trace_contributions/`
/// — the equivalent of `trace_contribution_dir_for_scope(None)` under an
/// arbitrary base — so the test never touches the real `~/.ironclaw/` tree.
/// The tempdir drops automatically; no manual cleanup is required.
#[tokio::test]
async fn instance_onboard_writes_instance_level_policy() {
    let base = tempfile::tempdir().expect("tempdir");
    // scope=None → trace_contributions/ directly under the base (no users/<hash>)
    let instance_dir = base.path().join("trace_contributions");

    // Use a loopback-shaped invite URL so origin anchoring passes; no server is
    // bound — the FakeSink returns a canned response without touching the network.
    let invite_url = "http://127.0.0.1:7/onboard#INVINST01";
    // Stage the pending key at the instance dir so canned_ok_body derives the
    // correct device_key_id that the client will cross-check.
    let body = canned_ok_body(&instance_dir, invite_url);
    let sink = FakeSink {
        status: 200,
        body,
        posted_url: Arc::new(Mutex::new(None)),
    };

    let outcome =
        onboard_at_dir_with_sink(&instance_dir, invite_url, OnboardConsents::default(), &sink)
            .await
            .expect("instance onboard succeeds");

    assert_eq!(outcome.tenant_id, "tenant-fake");

    // The policy must land at the scope-None location (no users/<hash> segment).
    let raw = std::fs::read_to_string(instance_dir.join("policy.json")).expect("policy written");
    let policy: StandingTraceContributionPolicy =
        serde_json::from_str(&raw).expect("policy parses");
    assert!(policy.enabled);
    assert_eq!(
        policy.device_key_id.as_deref(),
        Some(outcome.device_key_id.as_str()),
        "policy device_key_id must match outcome"
    );
    // tempdir drops automatically — no manual cleanup needed
}

/// The admin CLI entry point: enrollment via `onboard_instance_at_base` must
/// land the policy + device key at the INSTANCE location
/// (`<base>/trace_contributions/`, scope `None`) — not a `users/<hash>/`
/// scope dir — so every non-personally-enrolled user inherits it via
/// `resolve_trace_credentials` (inheritance itself is pinned by the resolver
/// tests in `contribution.rs`).
#[tokio::test]
async fn onboard_instance_at_base_targets_the_instance_dir() {
    let base = tempfile::tempdir().unwrap();
    let mock = spawn_mock_issuer(
        |addr| ok_response(addr, "https://ingest.example.com"),
        axum::http::StatusCode::OK,
    )
    .await;
    let invite_url = format!("http://127.0.0.1:{}/onboard#INVADMIN1", mock.addr.port());
    let consents = OnboardConsents {
        include_message_text: false,
        include_tool_payloads: false,
    };

    let outcome = onboard_instance_at_base(base.path(), &invite_url, consents)
        .await
        .expect("instance onboard succeeds");
    assert_eq!(outcome.tenant_id, "tenant-a");

    let instance_dir = base.path().join("trace_contributions");
    let policy_path = instance_dir.join("policy.json");
    assert!(
        policy_path.exists(),
        "policy must land at the instance (scope-None) location"
    );
    assert!(
        !base.path().join("trace_contributions/users").exists(),
        "instance enrollment must not create any per-user scope dir"
    );
    let policy: StandingTraceContributionPolicy =
        serde_json::from_str(&std::fs::read_to_string(&policy_path).unwrap()).unwrap();
    assert!(policy.enabled);
    assert_eq!(policy.auth_mode, TraceUploadAuthMode::DeviceKey);
    assert!(!policy.include_message_text);
    assert!(!policy.include_tool_payloads);

    // Promoted instance device key exists at the instance dir.
    let tenant_key = instance_dir.join(format!(
        "device_keys/{}.json",
        device_key::tenant_hash("tenant-a")
    ));
    assert!(tenant_key.exists(), "instance device key must be promoted");
}
