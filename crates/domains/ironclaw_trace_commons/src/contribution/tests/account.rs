//! The pinned contribution sinks, account login links, and account trace listing.

use crate::contribution::*;

use super::support::*;

#[tokio::test]
async fn mint_account_login_link_posts_subject_and_returns_url() {
    use std::sync::{Arc, Mutex};

    // A syntactically valid JWT that passes validate_trace_upload_claim_response.
    let claim_jwt = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "test-key-1"}));
    let claim_jwt_for_mock = claim_jwt.clone();

    // ── mock server ──────────────────────────────────────────────────────
    // Two endpoints:
    //   /v1/trace-upload-claim  — upload-claim issuer (reqwest, DeviceKey mode)
    //   /v1/account/login-links — the endpoint under test (via sink)
    let captured: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = captured.clone();

    let app = axum::Router::new()
        .route(
            "/v1/trace-upload-claim",
            axum::routing::post(move || {
                let jwt = claim_jwt_for_mock.clone();
                async move {
                    // Return a syntactically valid JWT so
                    // fetch_trace_upload_claim_from_issuer is satisfied.
                    axum::Json(serde_json::json!({
                        "access_token": jwt,
                        "token_type": "Bearer",
                        "expires_in": 300
                    }))
                }
            }),
        )
        .route(
            "/v1/account/login-links",
            axum::routing::post(move |axum::Json(b): axum::Json<serde_json::Value>| {
                let cap = cap.clone();
                async move {
                    cap.lock().unwrap().push(b);
                    axum::Json(serde_json::json!({
                        "account_id": "11111111-1111-1111-1111-111111111111",
                        "url": "/account/login?code=abc"
                    }))
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // ── isolated tempdir ─────────────────────────────────────────────────
    let base = tempfile::tempdir().unwrap();

    // Instance policy (scope None) — enables instance enrollment so
    // resolve_trace_credentials_at returns a per-user subject.
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
        upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
            "127.0.0.1".to_string()
        ]),
        upload_token_tenant_id: Some("tenant-dev".to_string()),
        upload_token_audience: Some("trace-commons-ingest".to_string()),
        ..Default::default()
    };
    write_trace_policy_for_scope_at(base.path(), None, &policy).expect("instance policy writes");

    // Generate and promote a device key at the instance scope dir so
    // DeviceKey auth mode can sign the workload JWT without a network call.
    let instance_dir = trace_contribution_dir_for_scope_at(base.path(), None);
    let pending =
        crate::onboarding::DeviceKeypair::load_or_generate_pending(&instance_dir, "testhash")
            .unwrap();
    pending.promote(&instance_dir, "tenant-dev").unwrap();

    // ── call under test ──────────────────────────────────────────────────
    let sink = RecordingSink::new();
    let link = mint_account_login_link_inner(base.path(), "tenant-dev", "alice", &sink)
        .await
        .unwrap();

    // ── assertions ───────────────────────────────────────────────────────
    // The server returned a RELATIVE url; it must come back absolutized
    // against the trust-anchored issuer origin, never left relative (a
    // relative URL would resolve against the consuming surface's origin).
    assert_eq!(link.url, format!("http://{addr}/account/login?code=abc"));
    assert_eq!(link.account_id, "11111111-1111-1111-1111-111111111111");

    // Egress invariant: on the agent path BOTH network calls — the
    // upload-claim mint and the login-link POST — must route through the
    // sink; a direct-reqwest claim mint would bypass RuntimeHttpEgress.
    {
        let sink_urls = sink.urls.lock().unwrap();
        assert_eq!(
            sink_urls.len(),
            2,
            "claim mint + login-link POST must both go through the sink; got {sink_urls:?}"
        );
        assert!(
            sink_urls[0].ends_with("/v1/trace-upload-claim"),
            "first sink request must be the upload-claim mint; got {sink_urls:?}"
        );
        assert!(
            sink_urls[1].ends_with("/v1/account/login-links"),
            "second sink request must be the login-link POST; got {sink_urls:?}"
        );
    }

    {
        let bodies = captured.lock().unwrap();
        assert_eq!(bodies.len(), 1, "exactly one POST to login-links");
        let expected_subject = salted_pseudonymous_contributor_id_at(
            base.path(),
            &trace_scope_key("tenant-dev", "alice"),
        )
        .unwrap();
        assert_eq!(
            bodies[0]["subject"],
            serde_json::Value::String(expected_subject),
            "posted subject must be per-user pseudonymous id for instance enrollment"
        );
    }

    // ── direct (WebUI service) variant ────────────────────────────────────
    // Same enrollment, no sink: the hosted-WebUI path mints through the
    // pinned direct client. The link is delivered ONLY in the return value
    // (the authenticated HTTP response) — it must never be persisted to a
    // local delivery file, which hosted users cannot read.
    let direct = mint_account_login_link_direct(base.path(), "tenant-dev", "alice")
        .await
        .expect("direct login-link mint succeeds");
    assert_eq!(direct.url, format!("http://{addr}/account/login?code=abc"));
    assert_eq!(direct.account_id, "11111111-1111-1111-1111-111111111111");
    let mut delivery_files = Vec::new();
    let mut stack = vec![base.path().to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("account_login_link."))
            {
                delivery_files.push(path);
            }
        }
    }
    assert!(
        delivery_files.is_empty(),
        "direct mint must not write a local delivery file; found {delivery_files:?}"
    );
    assert_eq!(
        captured.lock().unwrap().len(),
        2,
        "direct mint must POST to login-links too"
    );
}
/// The instance-aware profile-token mint (`*_for_user_*`) must resolve the
/// shared instance enrollment for a user with no personal-invite policy, and
/// carry that user's pseudonymous subject to the upload-claim issuer — else
/// instance-only contributors are falsely rejected as not enrolled.
#[tokio::test]
async fn mint_profile_attribution_token_for_user_uses_instance_subject() {
    use std::sync::{Arc, Mutex};

    let claim_jwt = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "test-key-1"}));
    let claim_jwt_for_mock = claim_jwt.clone();
    let claim_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let claim_cap = claim_bodies.clone();

    let app = axum::Router::new().route(
        "/v1/trace-upload-claim",
        axum::routing::post(move |axum::Json(b): axum::Json<serde_json::Value>| {
            let jwt = claim_jwt_for_mock.clone();
            let claim_cap = claim_cap.clone();
            async move {
                claim_cap.lock().unwrap().push(b);
                axum::Json(serde_json::json!({
                    "access_token": jwt,
                    "token_type": "Bearer",
                    "expires_in": 300
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let base = tempfile::tempdir().unwrap();
    enroll_instance_with_device_key(base.path(), addr);

    let sink = ReqwestContributionSink;
    let token =
        mint_profile_attribution_token_for_user_inner(base.path(), "tenant-dev", "alice", &sink)
            .await
            .expect("instance-enrolled user mints a profile-attribution token");
    assert_eq!(token.access_token, claim_jwt);

    let bodies = claim_bodies.lock().unwrap();
    assert_eq!(bodies.len(), 1, "exactly one claim request");
    let expected_subject =
        salted_pseudonymous_contributor_id_at(base.path(), &trace_scope_key("tenant-dev", "alice"))
            .unwrap();
    assert_eq!(
        bodies[0]["subject"],
        serde_json::Value::String(expected_subject),
        "claim request must carry the per-user pseudonymous subject for instance enrollment"
    );
}
/// The instance-aware community-profile publish (`*_for_user_*`) must resolve
/// the shared instance enrollment, mint under the per-user subject, and PUT
/// the profile — proving instance-only contributors can publish a profile.
#[tokio::test]
async fn set_community_profile_for_user_publishes_under_instance_subject() {
    use std::sync::{Arc, Mutex};

    let claim_jwt = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "test-key-1"}));
    let claim_jwt_for_mock = claim_jwt.clone();
    let claim_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let claim_cap = claim_bodies.clone();
    let profile_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let profile_cap = profile_bodies.clone();

    let app = axum::Router::new()
        .route(
            "/v1/trace-upload-claim",
            axum::routing::post(move |axum::Json(b): axum::Json<serde_json::Value>| {
                let jwt = claim_jwt_for_mock.clone();
                let claim_cap = claim_cap.clone();
                async move {
                    claim_cap.lock().unwrap().push(b);
                    axum::Json(serde_json::json!({
                        "access_token": jwt,
                        "token_type": "Bearer",
                        "expires_in": 300
                    }))
                }
            }),
        )
        .route(
            "/v1/community/profile",
            axum::routing::put(move |axum::Json(b): axum::Json<serde_json::Value>| {
                let profile_cap = profile_cap.clone();
                async move {
                    profile_cap.lock().unwrap().push(b);
                    axum::Json(serde_json::json!({ "ok": true }))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let base = tempfile::tempdir().unwrap();
    enroll_instance_with_device_key(base.path(), addr);

    let sink = ReqwestContributionSink;
    set_community_profile_for_user_inner(
        base.path(),
        "tenant-dev",
        "alice",
        "pilot_alice",
        Some("Trace Commons pilot"),
        Some(&sink),
    )
    .await
    .expect("instance-enrolled user publishes a community profile");

    let expected_subject =
        salted_pseudonymous_contributor_id_at(base.path(), &trace_scope_key("tenant-dev", "alice"))
            .unwrap();
    let claims = claim_bodies.lock().unwrap();
    assert_eq!(claims.len(), 1, "exactly one claim request");
    assert_eq!(
        claims[0]["subject"],
        serde_json::Value::String(expected_subject),
        "claim request must carry the per-user pseudonymous subject for instance enrollment"
    );
    let profiles = profile_bodies.lock().unwrap();
    assert_eq!(profiles.len(), 1, "exactly one community-profile PUT");
    assert_eq!(
        profiles[0]["display_handle"],
        serde_json::json!("pilot_alice")
    );
}
#[tokio::test]
async fn mint_account_login_link_pins_url_to_issuer_origin() {
    // Relative url: absolutized against the trust-anchored issuer origin.
    let (result, origin) = mint_login_link_with_response_url("/account/login?code=rel").await;
    let link = result.expect("relative url absolutizes");
    assert_eq!(link.url, format!("{origin}/account/login?code=rel"));

    // Cross-origin ABSOLUTE url: a hostile issuer response must not steer
    // the authenticated browser to another origin.
    let (result, _) =
        mint_login_link_with_response_url("https://attacker.example/account/login").await;
    let error = result.expect_err("cross-origin absolute url must be rejected");
    assert!(
        error.to_string().to_lowercase().contains("login link")
            || matches!(error, AccountLoginLinkError::Backend(_)),
        "cross-origin rejection surfaces as a Backend error: {error}"
    );

    // Non-HTTP(S) scheme: must be rejected (javascript: would execute in
    // the opened tab's context).
    let (result, _) = mint_login_link_with_response_url("javascript:alert(document.domain)").await;
    result.expect_err("non-http scheme must be rejected");

    // Userinfo smuggling: rejected even when the host would match. (The
    // mock's port isn't knowable before it binds, so a same-host+userinfo
    // URL can't be fabricated exactly — but userinfo is rejected before
    // the origin comparison, which cross-host coverage above pins anyway.)
    let (result, _) =
        mint_login_link_with_response_url("http://user:pass@127.0.0.1/account/login").await;
    result.expect_err("userinfo in the login-link url must be rejected");
}
#[tokio::test]
async fn direct_pinned_sink_rejects_private_hosts_and_bounds_bodies() {
    // Disallowed (link-local/metadata) host: rejected at resolution, before
    // any request is built.
    let error = DirectPinnedContributionSink
        .execute(ContributionHttpRequest {
            method: ContributionHttpMethod::Get,
            url: "http://169.254.169.254/v1/anything".to_string(),
            bearer_token: Some("secret".to_string()),
            json_body: None,
            response_body_limit: 1024,
            timeout_ms: 2_000,
        })
        .await
        .expect_err("link-local host must be rejected");
    assert!(
        error.to_string().contains("resolution rejected") || error.to_string().contains("rejected"),
        "rejection must come from host resolution: {error}"
    );

    // Redirects are NOT followed: the 3xx surfaces as the response status
    // and the Location target is never contacted.
    let hit = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hit_for_target = hit.clone();
    let target_app = axum::Router::new().route(
        "/stolen",
        axum::routing::get(move || {
            let hit = hit_for_target.clone();
            async move {
                hit.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                "leaked"
            }
        }),
    );
    let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_addr = target_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(target_listener, target_app).await;
    });
    let redirect_to = format!("http://{target_addr}/stolen");
    let redirect_app = axum::Router::new().route(
        "/hop",
        axum::routing::get(move || {
            let location = redirect_to.clone();
            async move {
                (
                    axum::http::StatusCode::FOUND,
                    [(axum::http::header::LOCATION, location)],
                    "",
                )
            }
        }),
    );
    let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect_addr = redirect_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(redirect_listener, redirect_app).await;
    });
    let response = DirectPinnedContributionSink
        .execute(ContributionHttpRequest {
            method: ContributionHttpMethod::Get,
            url: format!("http://{redirect_addr}/hop"),
            bearer_token: Some("secret".to_string()),
            json_body: None,
            response_body_limit: 1024,
            timeout_ms: 2_000,
        })
        .await
        .expect("redirect response surfaces, not followed");
    assert_eq!(response.status, 302, "3xx must surface as the status");
    assert_eq!(
        hit.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the redirect target must never be contacted"
    );

    // Oversized body: rejected DURING the read, not buffered.
    let big_app = axum::Router::new().route(
        "/big",
        axum::routing::get(|| async { "x".repeat(64 * 1024) }),
    );
    let big_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let big_addr = big_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(big_listener, big_app).await;
    });
    let error = DirectPinnedContributionSink
        .execute(ContributionHttpRequest {
            method: ContributionHttpMethod::Get,
            url: format!("http://{big_addr}/big"),
            bearer_token: None,
            json_body: None,
            response_body_limit: 1024,
            timeout_ms: 5_000,
        })
        .await
        .expect_err("oversized body must be rejected");
    assert!(
        error.to_string().contains("exceeds"),
        "rejection names the byte limit: {error}"
    );
}
#[tokio::test]
async fn mint_account_login_link_errors_when_not_enrolled() {
    let base = tempfile::tempdir().unwrap();
    // No policy written — resolver returns None.
    let sink = ReqwestContributionSink;
    let err = mint_account_login_link_inner(base.path(), "tenant-dev", "alice", &sink)
        .await
        .expect_err("unenrolled user must error");
    assert!(
        err.to_string().contains("not enrolled"),
        "error must mention enrollment: {err}"
    );
}
#[test]
fn account_login_links_url_errors_on_wrong_suffix() {
    // URL that does NOT end in /v1/trace-upload-claim — must error, not silently misroute.
    let policy = StandingTraceContributionPolicy {
        upload_token_issuer_url: Some("https://api.example.com/v2/trace-upload-claim".to_string()),
        ..Default::default()
    };
    let err = account_login_links_url(&policy).expect_err("wrong suffix must be an error");
    assert!(
        err.to_string()
            .contains("does not end in /v1/trace-upload-claim"),
        "error must name the expected suffix: {err}"
    );
}
#[test]
fn account_login_links_url_correct_on_valid_issuer() {
    let policy = StandingTraceContributionPolicy {
        upload_token_issuer_url: Some("https://api.example.com/v1/trace-upload-claim".to_string()),
        ..Default::default()
    };
    let url = account_login_links_url(&policy).expect("valid issuer must succeed");
    assert_eq!(url, "https://api.example.com/v1/account/login-links");
}
#[test]
fn account_traces_url_correct_with_and_without_limit() {
    let policy = StandingTraceContributionPolicy {
        upload_token_issuer_url: Some("https://api.example.com/v1/trace-upload-claim".to_string()),
        ..Default::default()
    };
    // None defaults to the bounded page size (never an unbounded fetch).
    let url_no_limit = account_traces_url(&policy, None).expect("no-limit must succeed");
    assert_eq!(
        url_no_limit,
        format!(
            "https://api.example.com/v1/account/traces?limit={}",
            ACCOUNT_TRACES_DEFAULT_LIMIT
        )
    );
    let url_with_limit = account_traces_url(&policy, Some(50)).expect("limit=50 must succeed");
    assert_eq!(
        url_with_limit,
        "https://api.example.com/v1/account/traces?limit=50"
    );
    // An over-large limit is clamped to the hard ceiling.
    let url_clamped = account_traces_url(&policy, Some(100_000)).expect("large limit must succeed");
    assert_eq!(
        url_clamped,
        format!(
            "https://api.example.com/v1/account/traces?limit={}",
            ACCOUNT_TRACES_MAX_LIMIT
        )
    );
}
#[tokio::test]
async fn fetch_account_traces_returns_user_submissions() {
    // A syntactically valid JWT that passes validate_trace_upload_claim_response.
    let claim_jwt = test_jwt_with_header(serde_json::json!({"alg": "EdDSA", "kid": "test-key-1"}));
    let claim_jwt_for_mock = claim_jwt.clone();

    // ── mock server ──────────────────────────────────────────────────────
    // Two endpoints:
    //   /v1/trace-upload-claim  — upload-claim issuer (DeviceKey mode)
    //   /v1/account/traces      — the endpoint under test (via sink)
    let app = axum::Router::new()
        .route(
            "/v1/trace-upload-claim",
            axum::routing::post(move || {
                let jwt = claim_jwt_for_mock.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "access_token": jwt,
                        "token_type": "Bearer",
                        "expires_in": 300
                    }))
                }
            }),
        )
        .route(
            "/v1/account/traces",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!([
                    {
                        "submission_id": "s1",
                        "status": "accepted",
                        "credit_points_pending": 1.0,
                        "credit_points_final": 1.0,
                        "received_at": "2026-06-25T00:00:00Z"
                    }
                ]))
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // ── isolated tempdir ─────────────────────────────────────────────────
    let base = tempfile::tempdir().unwrap();

    // Instance policy (scope None) — enables instance enrollment so
    // resolve_trace_credentials_at returns a per-user subject.
    let policy = StandingTraceContributionPolicy {
        enabled: true,
        auth_mode: TraceUploadAuthMode::DeviceKey,
        upload_token_issuer_url: Some(format!("http://{addr}/v1/trace-upload-claim")),
        upload_token_issuer_allowed_hosts: std::collections::BTreeSet::from([
            "127.0.0.1".to_string()
        ]),
        upload_token_tenant_id: Some("tenant-dev".to_string()),
        upload_token_audience: Some("trace-commons-ingest".to_string()),
        ..Default::default()
    };
    write_trace_policy_for_scope_at(base.path(), None, &policy).expect("instance policy writes");

    // Generate and promote a device key at the instance scope dir so
    // DeviceKey auth mode can sign the workload JWT without a network call.
    let instance_dir = trace_contribution_dir_for_scope_at(base.path(), None);
    let pending =
        crate::onboarding::DeviceKeypair::load_or_generate_pending(&instance_dir, "testhash")
            .unwrap();
    pending.promote(&instance_dir, "tenant-dev").unwrap();

    // ── call under test ──────────────────────────────────────────────────
    let sink = RecordingSink::new();
    let items = fetch_account_traces_inner(base.path(), "tenant-dev", "alice", Some(50), &sink)
        .await
        .unwrap();

    // ── assertions ───────────────────────────────────────────────────────
    // Egress invariant: claim mint + traces GET must both route through
    // the sink on the agent path (no direct-reqwest claim mint).
    {
        let sink_urls = sink.urls.lock().unwrap();
        assert_eq!(
            sink_urls.len(),
            2,
            "claim mint + traces GET must both go through the sink; got {sink_urls:?}"
        );
        assert!(
            sink_urls[0].ends_with("/v1/trace-upload-claim"),
            "first sink request must be the upload-claim mint; got {sink_urls:?}"
        );
        assert!(
            sink_urls[1].contains("/v1/account/traces"),
            "second sink request must be the traces GET; got {sink_urls:?}"
        );
    }
    assert_eq!(items.len(), 1, "expected exactly one trace item");
    assert_eq!(items[0].submission_id, "s1");
    assert_eq!(items[0].status, "accepted");
    assert!(
        (items[0].credit_points_pending - 1.0).abs() < f32::EPSILON,
        "credit_points_pending must be 1.0"
    );
    assert_eq!(
        items[0].credit_points_final,
        Some(1.0),
        "credit_points_final must be Some(1.0)"
    );
    assert_eq!(
        items[0].received_at.as_deref(),
        Some("2026-06-25T00:00:00Z")
    );
}
#[tokio::test]
async fn fetch_account_traces_returns_empty_when_not_enrolled() {
    let base = tempfile::tempdir().unwrap();
    // No policy written — resolver returns None → lenient Ok(vec![]).
    let sink = ReqwestContributionSink;
    let items = fetch_account_traces_inner(base.path(), "tenant-dev", "alice", None, &sink)
        .await
        .unwrap();
    assert!(items.is_empty(), "unenrolled user must return empty list");
}
#[tokio::test]
async fn fetch_account_traces_errors_on_server_error() {
    // A 5xx must NOT be swallowed as an empty list — it surfaces as Err so
    // the WebUI boundary renders a sanitized unavailable state. Both the
    // sink-backed (agent) and direct (WebUI/CLI) paths must agree.
    let (via_sink, direct) = fetch_account_traces_with_status(
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        serde_json::json!({"error": "boom"}),
    )
    .await;
    assert!(
        via_sink.is_err(),
        "sink path: 5xx must surface as an error, not empty"
    );
    assert!(
        direct.is_err(),
        "direct path: 5xx must surface as an error, not empty"
    );
}
#[tokio::test]
async fn fetch_account_traces_404_is_empty() {
    // 404 = no account/traces yet for this enrolled principal → legitimate
    // empty state, not an error. Both fetch paths must agree.
    let (via_sink, direct) = fetch_account_traces_with_status(
        axum::http::StatusCode::NOT_FOUND,
        serde_json::json!({"error": "no account"}),
    )
    .await;
    assert!(
        via_sink
            .expect("sink path: 404 must be the empty zero-state")
            .is_empty()
    );
    assert!(
        direct
            .expect("direct path: 404 must be the empty zero-state")
            .is_empty()
    );
}
#[tokio::test]
async fn pinned_trace_remote_client_rejects_private_endpoint_hosts() {
    // The background submit/status/revoke lane pins DNS per request: a host
    // resolving to a private/link-local address must be rejected before any
    // bearer-authenticated request is built (DNS-rebinding defense).
    //
    // Since #7144 the endpoint is also validated in the builder, so a
    // link-local literal is now refused one step earlier — as `Endpoint`
    // rather than `NetworkDns`. Still rejected, and rejected sooner; the
    // assertion follows the stronger guard rather than pinning the weaker
    // one.
    let error = pinned_trace_remote_http_client("http://169.254.169.254/v1/traces")
        .await
        .expect_err("link-local endpoint host must be rejected");
    assert_eq!(error.kind, TraceQueueTelemetryFailureKind::Endpoint);

    // The literal-loopback standalone exception still applies.
    pinned_trace_remote_http_client("http://127.0.0.1:8080/v1/traces")
        .await
        .expect("literal loopback endpoint builds (standalone exception)");
}

/// #7144: the builder attaches the enrolled bearer to whatever endpoint the
/// policy carries, and `ironclaw traces opt-in --endpoint <url>` writes that
/// endpoint unvalidated. Before this, `http://` to a public host built a
/// client happily and the token went out in clear text — the comment above
/// the builder claimed a validator ran on this lane, and none did.
#[tokio::test]
async fn pinned_trace_remote_client_refuses_plaintext_http_to_a_public_host() {
    let error = pinned_trace_remote_http_client("http://traces.example.test/v1/traces")
        .await
        .expect_err("a bearer must never be attached to a plaintext public endpoint");
    assert_eq!(error.kind, TraceQueueTelemetryFailureKind::Endpoint);
    assert!(
        error.to_string().contains("https"),
        "the refusal must say why: {error}"
    );

    // The guard is about the scheme, not the host, so it must not have
    // become a blanket refusal: the same host over https gets past the
    // endpoint validator and fails later, at the DNS pin. Asserted as "not
    // Endpoint" rather than "builds", because `.test` never resolves — a
    // success assertion here would depend on the runner having a network.
    let https_error = pinned_trace_remote_http_client("https://traces.example.test/v1/traces")
        .await
        .expect_err("an unresolvable host still fails, but later");
    assert_eq!(
        https_error.kind,
        TraceQueueTelemetryFailureKind::NetworkDns,
        "https must clear the endpoint guard and reach DNS pinning, got: {https_error}"
    );
}
