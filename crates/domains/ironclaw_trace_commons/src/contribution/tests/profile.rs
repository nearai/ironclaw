//! Community-profile validation and the profile PUT/DELETE wire contract.

use std::collections::BTreeSet;
use std::sync::Arc;

use uuid::Uuid;

use crate::contribution::*;

use super::support::*;

#[test]
fn community_profile_handle_validation_rejects_bad_handles() {
    let error = validate_community_profile_handle("ab").expect_err("too short rejected");
    assert!(error.to_string().contains("at least 3"));

    let long = "a".repeat(33);
    let error = validate_community_profile_handle(&long).expect_err("too long rejected");
    assert!(error.to_string().contains("at most 32"));

    for bad in ["bad handle!", "naïve", "pilot.zaki", "pilot/zaki"] {
        let error = validate_community_profile_handle(bad).expect_err("bad characters rejected");
        assert!(
            error.to_string().contains("ASCII letters"),
            "{bad} should fail the character-set check: {error}"
        );
    }

    assert_eq!(
        validate_community_profile_handle("  pilot_zaki  ").expect("trimmed handle accepted"),
        "pilot_zaki"
    );
    assert_eq!(
        validate_community_profile_handle("Pilot-Zaki_42").expect("alnum/-/_ accepted"),
        "Pilot-Zaki_42"
    );
}
#[test]
fn community_profile_bio_validation_bounds_bytes() {
    validate_community_profile_bio(&"x".repeat(280)).expect("280 bytes accepted");
    let error = validate_community_profile_bio(&"x".repeat(281)).expect_err("281 bytes rejected");
    assert!(error.to_string().contains("at most 280 bytes"));
    // Byte-length, not char-length: 141 two-byte chars = 282 bytes.
    assert!(validate_community_profile_bio(&"é".repeat(141)).is_err());
}
#[test]
fn community_profile_url_derives_from_ingest_url() {
    let policy = StandingTraceContributionPolicy::default()
        .set_ingestion_endpoint("https://ingest.example.com:8443/v1/traces".to_string())
        .set_upload_token_issuer_url("https://issuer.example.com/v1/trace-upload-claim".to_string())
        .set_upload_token_issuer_allowed_hosts(BTreeSet::from(["issuer.example.com".to_string()]));
    let url = community_profile_url_from_policy(&policy).expect("profile URL derives");
    assert_eq!(
        url.as_str(),
        "https://ingest.example.com:8443/v1/community/profile",
        "scheme/host/port preserved, path replaced"
    );

    // Profile routing must not depend on issuer host compatibility.
    let split_hosts = StandingTraceContributionPolicy::default()
        .set_ingestion_endpoint("https://ingest.tracecommons.ai/v1/traces".to_string())
        .set_upload_token_issuer_url(
            "https://issuer.tracecommons.ai/v1/trace-upload-claim".to_string(),
        )
        .set_upload_token_issuer_allowed_hosts(BTreeSet::from([
            "issuer.tracecommons.ai".to_string()
        ]));
    let split_url = community_profile_url_from_policy(&split_hosts).expect("split hosts derive");
    assert_eq!(
        split_url.as_str(),
        "https://ingest.tracecommons.ai/v1/community/profile"
    );

    // Plain HTTP ingest endpoints are rejected.
    let insecure = StandingTraceContributionPolicy::default()
        .set_ingestion_endpoint("http://ingest.example.com/v1/traces".to_string());
    assert!(community_profile_url_from_policy(&insecure).is_err());

    // Internal (non-loopback) ingest hosts are rejected.
    let internal = StandingTraceContributionPolicy::default()
        .set_ingestion_endpoint("https://ingest.corp.internal/v1/traces".to_string());
    assert!(community_profile_url_from_policy(&internal).is_err());

    // Literal loopback gets the dev exception (loopback-HTTP onboarding
    // stores a loopback ingest endpoint).
    let loopback = StandingTraceContributionPolicy::default()
        .set_ingestion_endpoint("http://127.0.0.1:3917/v1/traces".to_string());
    let loopback_url =
        community_profile_url_from_policy(&loopback).expect("loopback dev ingest derives");
    assert_eq!(
        loopback_url.as_str(),
        "http://127.0.0.1:3917/v1/community/profile"
    );

    // A mounted prefix on the ingest path must be preserved (mirrors
    // trace_submission_status_endpoint), not clobbered to the bare path.
    let prefixed = StandingTraceContributionPolicy::default()
        .set_ingestion_endpoint("https://ingest.example.com/api/v1/traces".to_string());
    assert_eq!(
        community_profile_url_from_policy(&prefixed)
            .expect("prefixed ingest derives")
            .as_str(),
        "https://ingest.example.com/api/v1/community/profile"
    );
}
#[tokio::test]
async fn mint_profile_attribution_token_requires_enrollment() {
    let scope = format!("trace-profile-test-{}", Uuid::new_v4());
    write_trace_policy_for_scope(Some(&scope), &StandingTraceContributionPolicy::default())
        .expect("policy writes");
    let error = mint_profile_attribution_token_for_scope(Some(&scope))
        .await
        .expect_err("disabled policy must refuse to mint");
    assert!(
        error.to_string().contains("not enrolled in Trace Commons"),
        "error must point at onboarding: {error}"
    );
    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn mint_profile_attribution_token_requires_issuer_url() {
    let scope = format!("trace-profile-test-{}", Uuid::new_v4());
    write_trace_policy_for_scope(
        Some(&scope),
        &StandingTraceContributionPolicy::default().set_enabled(true),
    )
    .expect("policy writes");
    let error = mint_profile_attribution_token_for_scope(Some(&scope))
        .await
        .expect_err("missing issuer URL must refuse to mint");
    assert!(
        error.to_string().contains("issuer URL is not configured"),
        "error must name the missing issuer URL: {error}"
    );
    let _ = std::fs::remove_dir_all(trace_contribution_dir_for_scope(Some(&scope)));
}
#[tokio::test]
async fn profile_attribution_claim_request_wire_shape_and_mock_issuer_roundtrip() {
    // Like the PilotAllowlist tests above, we assert the wire shape via
    // the factored-out request builder and confirm a real issuer
    // round-trip of exactly that body against a mock that rejects any
    // drift (the full fetch path is covered separately by
    // fetch_trace_upload_claim_from_issuer_accepts_loopback_dev_issuer).
    let scope = format!("trace-profile-test-{}", Uuid::new_v4());
    let context = profile_attribution_claim_context(Some(&scope));
    let policy = StandingTraceContributionPolicy::default()
        .set_auth_mode(TraceUploadAuthMode::WorkloadTokenEnv)
        .set_upload_token_tenant_id("tenant-a".to_string())
        .set_upload_token_audience("trace-commons".to_string());
    let request = build_trace_upload_claim_issuer_request(&policy, &context);
    let body = serde_json::to_value(&request).expect("request serializes");
    assert_eq!(
        body["consent_scopes"],
        serde_json::json!(["public_attribution"])
    );
    let obj = body.as_object().expect("request body is an object");
    assert!(
        !obj.contains_key("allowed_uses"),
        "empty allowed_uses must be skip-serialized"
    );
    assert!(
        !obj.contains_key("trace_id"),
        "profile claims carry no trace_id"
    );
    assert!(
        !obj.contains_key("submission_id"),
        "profile claims carry no submission_id"
    );

    let mint_token = test_jwt_with_header(serde_json::json!({
        "alg": "EdDSA",
        "kid": "managed-key-1"
    }));
    let mint_token_for_route = mint_token.clone();
    let app = axum::Router::new().route(
        "/v1/trace-upload-claim",
        axum::routing::post(
            move |axum::Json(request_body): axum::Json<serde_json::Value>| {
                let mint_token = mint_token_for_route.clone();
                async move {
                    let obj = request_body.as_object().cloned().unwrap_or_default();
                    if request_body["consent_scopes"] != serde_json::json!(["public_attribution"])
                        || obj.contains_key("allowed_uses")
                        || obj.contains_key("trace_id")
                        || obj.contains_key("submission_id")
                    {
                        return (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::Json(serde_json::json!({"error": "unexpected claim body"})),
                        );
                    }
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "access_token": mint_token,
                            "token_type": "Bearer",
                            "expires_in": 300
                        })),
                    )
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock issuer listener binds");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::builder()
        .build()
        .expect("reqwest client builds");
    let response = client
        .post(format!("http://{addr}/v1/trace-upload-claim"))
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&request)
        .send()
        .await
        .expect("mock issuer responds");
    assert_eq!(
        response.status().as_u16(),
        200,
        "mock issuer must accept the profile claim body"
    );
    let claim: TraceUploadClaimIssuerResponse =
        response.json().await.expect("claim response parses");
    validate_trace_upload_claim_response(&claim).expect("mock claim passes validation");
    assert_eq!(claim.access_token, mint_token);
    assert_eq!(claim.expires_in, Some(300));
}
#[tokio::test]
async fn community_profile_put_sends_bearer_and_body() {
    let token = test_jwt_with_header(serde_json::json!({
        "alg": "EdDSA",
        "kid": "managed-key-1"
    }));
    let seen: Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_for_route = seen.clone();
    let app = axum::Router::new().route(
        "/v1/community/profile",
        axum::routing::put(
            move |headers: axum::http::HeaderMap,
                  axum::Json(body): axum::Json<serde_json::Value>| {
                let seen = seen_for_route.clone();
                async move {
                    let authorization = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("<missing>")
                        .to_string();
                    seen.lock().expect("seen lock").push((authorization, body));
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(serde_json::json!({"display_handle": "pilot_zaki"})),
                    )
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock profile listener binds");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let url = reqwest::Url::parse(&format!("http://{addr}/v1/community/profile"))
        .expect("profile url parses");
    // None sink exercises the worker/CLI crate-local reqwest path
    // (community_profile_http_client builds against the loopback mock).
    let policy = StandingTraceContributionPolicy::default();
    execute_community_profile_request(
        &policy,
        ContributionHttpMethod::Put,
        url,
        &token,
        Some(&serde_json::json!({"display_handle": "pilot_zaki", "bio": null})),
        None,
    )
    .await
    .expect("profile PUT succeeds");

    let seen = seen.lock().expect("seen lock");
    assert_eq!(seen.len(), 1);
    let (authorization, body) = &seen[0];
    assert_eq!(authorization, &format!("Bearer {token}"));
    assert_eq!(
        body,
        &serde_json::json!({"display_handle": "pilot_zaki", "bio": null})
    );
}
#[tokio::test]
async fn community_profile_delete_sends_bearer_without_body() {
    let token = test_jwt_with_header(serde_json::json!({
        "alg": "EdDSA",
        "kid": "managed-key-1"
    }));
    let seen: Arc<std::sync::Mutex<Vec<(String, usize)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_for_route = seen.clone();
    let app = axum::Router::new().route(
        "/v1/community/profile",
        axum::routing::delete(
            move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                let seen = seen_for_route.clone();
                async move {
                    let authorization = headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("<missing>")
                        .to_string();
                    seen.lock()
                        .expect("seen lock")
                        .push((authorization, body.len()));
                    axum::http::StatusCode::NO_CONTENT
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock profile listener binds");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let url = reqwest::Url::parse(&format!("http://{addr}/v1/community/profile"))
        .expect("profile url parses");
    let policy = StandingTraceContributionPolicy::default();
    execute_community_profile_request(
        &policy,
        ContributionHttpMethod::Delete,
        url,
        &token,
        None,
        None,
    )
    .await
    .expect("profile DELETE succeeds");

    let seen = seen.lock().expect("seen lock");
    assert_eq!(seen.len(), 1);
    let (authorization, body_len) = &seen[0];
    assert_eq!(authorization, &format!("Bearer {token}"));
    assert_eq!(*body_len, 0, "withdraw must send no body");
}
#[tokio::test]
async fn community_profile_error_surfaces_bounded_error_field_without_token() {
    let token = test_jwt_with_header(serde_json::json!({
        "alg": "EdDSA",
        "kid": "managed-key-1"
    }));
    let app = axum::Router::new().route(
        "/v1/community/profile",
        axum::routing::put(|| async {
            (
                axum::http::StatusCode::CONFLICT,
                axum::Json(serde_json::json!({"error": "display handle already taken"})),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock profile listener binds");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let url = reqwest::Url::parse(&format!("http://{addr}/v1/community/profile"))
        .expect("profile url parses");
    let policy = StandingTraceContributionPolicy::default();
    let error = execute_community_profile_request(
        &policy,
        ContributionHttpMethod::Put,
        url,
        &token,
        Some(&serde_json::json!({"display_handle": "pilot_zaki", "bio": null})),
        None,
    )
    .await
    .expect_err("conflict must surface as an error");

    let chain = format!("{error:#}");
    assert!(chain.contains("HTTP 409"), "status surfaces: {chain}");
    assert!(
        chain.contains("display handle already taken"),
        "bounded error field surfaces: {chain}"
    );
    assert!(
        !chain.contains(&token),
        "bearer token must never appear in errors: {chain}"
    );
}
#[tokio::test]
async fn set_community_profile_rejects_invalid_handle_before_any_network_call() {
    // Test through the caller: the public entrypoint must refuse a bad
    // handle before reading policy or touching the network.
    let scope = format!("trace-profile-test-{}", Uuid::new_v4());
    let error = set_community_profile_for_scope(Some(&scope), "x", None)
        .await
        .expect_err("short handle must be rejected");
    assert!(error.to_string().contains("at least 3"));

    let error = set_community_profile_for_scope(Some(&scope), "ok_handle", Some(&"x".repeat(281)))
        .await
        .expect_err("oversized bio must be rejected");
    assert!(error.to_string().contains("at most 280 bytes"));
}
