//! End-to-end test: agent invokes `builtin.trace_commons.onboard` and
//! `builtin.trace_commons.status` through the host-runtime capability-dispatch
//! path (not directly through the library).
//!
//! This file is a separate test binary so it gets a fresh process — and
//! therefore a fresh `LazyLock` for `ironclaw_base_dir()`. `IRONCLAW_BASE_DIR`
//! is set to a tempdir as the very first action so the LazyLock picks it up.
//!
//! All tests share the tempdir (base dir is process-wide); they use different
//! mock ports / invite codes and, critically, a distinct per-test user scope
//! (passed into `execution_context_with_network`) so onboarding state written
//! by one test cannot bleed into another running concurrently.

#[path = "support/trace_commons_dispatch.rs"]
mod tc_support;

use ironclaw_host_runtime::{
    TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID, TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
    TRACE_COMMONS_STATUS_CAPABILITY_ID,
};
use serde_json::json;
use tc_support::*;

// ── Tests ────────────────────────────────────────────────────────────────────

/// Verify the full dispatch chain:
///   agent → `builtin.trace_commons.onboard` (confirmed=true)
///         → host runtime dispatch
///         → `ironclaw_reborn_traces::onboarding::onboard()`
///         → real HTTP POST to mock issuer
///         → policy written
///   then `builtin.trace_commons.status` reports enrolled.
#[tokio::test]
async fn onboard_then_status_through_dispatch() {
    let _base_dir = setup_base_dir();

    let (addr, received) = spawn_mock_issuer(
        |addr| {
            json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-e2e",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
                "profile_url": "https://tracecommons.ai/profile",
                "leaderboard_url": "https://tracecommons.ai/lb",
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;

    let invite_url = format!("http://127.0.0.1:{}/onboard#INVE2E001", addr.port());
    let rt = runtime();

    // ── invoke onboard ────────────────────────────────────────────────────────
    let onboard_result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        json!({
            "invite_url": invite_url,
            "include_message_text": true,
            "include_tool_payloads": false,
            "confirmed": true,
        }),
        execution_context_with_network(
            "user_onboard_then_status",
            "caller_onboard_then_status",
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("onboard dispatch must succeed (Ok variant)");

    // ── assert onboard response fields ────────────────────────────────────────
    assert_eq!(
        onboard_result["enrolled"],
        json!(true),
        "enrolled must be true after successful onboard"
    );
    assert_eq!(
        onboard_result["tenant_id"],
        json!("tenant-e2e"),
        "tenant_id must match mock response"
    );
    let device_key_id = onboard_result["device_key_id"]
        .as_str()
        .expect("device_key_id must be a string");
    assert!(
        device_key_id.starts_with("sha256:"),
        "device_key_id must start with 'sha256:'"
    );
    assert_eq!(
        onboard_result["consents"]["include_message_text"],
        json!(true),
        "include_message_text consent must be recorded"
    );
    assert_eq!(
        onboard_result["consents"]["include_tool_payloads"],
        json!(false),
        "include_tool_payloads consent must be recorded"
    );

    // ── assert mock received exactly 1 request with expected fields ───────────
    {
        let requests = received.lock().unwrap();
        assert_eq!(requests.len(), 1, "mock must receive exactly 1 POST");
        let req = &requests[0];
        let pubkey = req["device_public_key"]
            .as_str()
            .expect("device_public_key must be present");
        assert!(!pubkey.is_empty(), "device_public_key must not be empty");
        assert_eq!(
            req["invite_code"],
            json!("INVE2E001"),
            "invite_code must match the fragment in the invite URL"
        );
    }

    // ── invoke status ─────────────────────────────────────────────────────────
    let status_result = invoke_with_context(
        &rt,
        TRACE_COMMONS_STATUS_CAPABILITY_ID,
        json!({}),
        execution_context_read_only(
            "user_onboard_then_status",
            "caller_onboard_then_status",
            TRACE_COMMONS_STATUS_CAPABILITY_ID,
        ),
    )
    .await
    .expect("status dispatch must succeed");

    assert_eq!(
        status_result["enrolled"],
        json!(true),
        "status must report enrolled after onboard"
    );
    assert_eq!(
        status_result["tenant_id"],
        json!("tenant-e2e"),
        "status tenant_id must match"
    );
    assert_eq!(
        status_result["auth_mode"],
        json!("device_key"),
        "auth_mode must be device_key"
    );
    assert_eq!(
        status_result["include_message_text"],
        json!(true),
        "status include_message_text must reflect consents"
    );
}

/// #4560: with the production-default network policy (private/loopback IP
/// ranges denied), the agent onboard POST to a 127.0.0.1 invite must be blocked
/// by the host network-egress policy — the tool reports a network failure and
/// does NOT enroll. This is the regression test demonstrating the fix: the agent
/// can no longer reach private destinations through onboarding.
#[tokio::test]
async fn onboard_private_ip_blocked_by_network_policy() {
    let _base_dir = setup_base_dir();

    let (addr, received) = spawn_mock_issuer(
        |addr| {
            json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-blocked",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;

    let invite_url = format!("http://127.0.0.1:{}/onboard#INVE2E003", addr.port());
    let rt = runtime();

    let result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        json!({
            "invite_url": invite_url,
            "include_message_text": false,
            "include_tool_payloads": false,
            "confirmed": true,
        }),
        execution_context_with_network(
            "user_private_ip_blocked",
            "caller_private_ip_blocked",
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
            deny_private_ip_network_policy(),
        ),
    )
    .await
    .expect("onboard dispatch returns an Ok envelope even when egress is blocked");

    assert_ne!(
        result["enrolled"],
        json!(true),
        "onboard must NOT enroll when the network policy blocks the private-IP destination"
    );
    assert_eq!(
        result["error_code"],
        json!("Network"),
        "blocked egress must surface as a network failure (invite not consumed)"
    );

    // The policy blocks before any bytes reach the wire: the mock must see no
    // request body parsed (the egress denies the private-IP target pre-flight).
    let requests = received.lock().unwrap();
    assert_eq!(
        requests.len(),
        0,
        "no onboarding POST may reach the private-IP destination once the policy denies it"
    );
}

/// Verify that `confirmed=false` short-circuits before making ANY network call.
#[tokio::test]
async fn onboard_unconfirmed_makes_no_network_call() {
    let _base_dir = setup_base_dir();

    let (addr, received) = spawn_mock_issuer(
        |addr| {
            json!({
                "schema_version": "trace_commons.onboard_response.v1",
                "tenant_id": "tenant-unconfirmed",
                "ingest_url": "https://ingest.example.com",
                "issuer_url": format!("http://127.0.0.1:{}", addr.port()),
                "audience": "trace-commons-ingest",
                "device_key_id": ECHO_DEVICE_KEY_ID,
            })
        },
        axum::http::StatusCode::OK,
    )
    .await;

    let invite_url = format!("http://127.0.0.1:{}/onboard#INVE2E002", addr.port());
    let rt = runtime();

    let result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        json!({
            "invite_url": invite_url,
            "include_message_text": false,
            "include_tool_payloads": false,
            "confirmed": false,
        }),
        execution_context_with_network(
            "user_unconfirmed_no_network",
            "caller_unconfirmed_no_network",
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("unconfirmed dispatch must succeed (Ok variant with consent gate)");

    // The consent gate returns enrolled=false + consent_required=true.
    assert_eq!(
        result["enrolled"],
        json!(false),
        "enrolled must be false for unconfirmed call"
    );
    assert_eq!(
        result["consent_required"],
        json!(true),
        "consent_required must be true"
    );

    // The mock must have received ZERO requests — no network call made.
    let requests = received.lock().unwrap();
    assert_eq!(
        requests.len(),
        0,
        "no HTTP requests must reach the mock when confirmed=false"
    );
}

/// Verify the consent gate: `account_login_link` with no `confirmed` must
/// return `consent_required=true` without making any network call.
#[tokio::test]
async fn account_login_link_requires_consent() {
    let _base_dir = setup_base_dir();

    let rt = runtime();

    // Use allow_all_network_policy because the capability manifest declares
    // EffectKind::Network; the host runtime stages the network grant before
    // dispatching. The consent gate short-circuits inside the handler before
    // making any actual network call.
    let result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
        json!({}),
        execution_context_with_network(
            "user_login_link_consent",
            "caller_login_link_consent",
            TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("consent-gate dispatch must succeed (Ok envelope, not an Err)");

    assert_eq!(
        result["minted"],
        json!(false),
        "minted must be false when confirmed is absent"
    );
    assert_eq!(
        result["consent_required"],
        json!(true),
        "consent_required must be true when confirmed is absent"
    );
    assert!(
        result["message"].as_str().is_some_and(|m| !m.is_empty()),
        "message must be non-empty"
    );
}

/// Build a syntactically valid JWT string with the given header object.
/// The signature segment is a literal ASCII placeholder — JWT validation
/// in this codebase only inspects the header fields (alg, kid) and the
/// presence of a non-empty access_token, so this is sufficient for tests.
/// Verify the full dispatch chain for account_login_link:
///   1. Onboard via mock server (writes local enrollment state).
///   2. Agent invokes `builtin.trace_commons.account_login_link` with confirmed=true.
///   3. The tool fetches a bearer token from `/v1/trace-upload-claim` (reqwest path).
///   4. The tool POSTs to `/v1/account/login-links` (via host egress sink).
///   5. The tool returns `minted=true` and the login URL.
#[tokio::test]
async fn account_login_link_through_dispatch() {
    let _base_dir = setup_base_dir();

    let claim_jwt = test_jwt_eddsa("e2e-key-1");
    let claim_jwt_for_mock = claim_jwt.clone();

    // Spawn a mock server that handles all three routes needed:
    //   /v1/onboard              — onboarding POST (standard mock response)
    //   /v1/trace-upload-claim   — bearer-token issuer (reqwest path, not sink)
    //   /v1/account/login-links  — the endpoint under test (via host egress sink)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock server binds");
    let addr = listener.local_addr().expect("mock server local addr");

    let app = {
        let claim_jwt_handler = claim_jwt_for_mock.clone();
        let addr_for_onboard = addr;
        axum::Router::new()
            .route(
                "/v1/onboard",
                axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                    let port = addr_for_onboard.port();
                    async move {
                        // Echo the device_key_id from the submitted public key —
                        // mirrors spawn_mock_issuer's ECHO_DEVICE_KEY_ID logic.
                        let pubkey_b64 = body["device_public_key"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        let device_key_id = derive_device_key_id(&pubkey_b64)
                            .unwrap_or_else(|| "sha256:unknown".to_string());
                        axum::Json(serde_json::json!({
                            "schema_version": "trace_commons.onboard_response.v1",
                            "tenant_id": "tenant-login-link",
                            "ingest_url": "https://ingest.example.com",
                            "issuer_url": format!("http://127.0.0.1:{}/v1/trace-upload-claim", port),
                            "audience": "trace-commons-ingest",
                            "device_key_id": device_key_id,
                        }))
                    }
                }),
            )
            .route(
                "/v1/trace-upload-claim",
                axum::routing::post(move || {
                    let jwt = claim_jwt_handler.clone();
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
                "/v1/account/login-links",
                axum::routing::post(|| async {
                    axum::Json(serde_json::json!({
                        "account_id": "acc123",
                        "url": "/account/login?code=testcode123"
                    }))
                }),
            )
    };

    tokio::spawn(async move {
        #[allow(clippy::let_underscore_must_use)]
        // Background test server; the serve result is unused for test lifetime.
        let _ = axum::serve(listener, app).await;
    });

    let invite_url = format!("http://127.0.0.1:{}/onboard#LOGINLINKE2E", addr.port());
    let rt = runtime();

    // ── Step 1: Onboard ───────────────────────────────────────────────────────
    let onboard_result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        json!({
            "invite_url": invite_url,
            "include_message_text": false,
            "include_tool_payloads": false,
            "confirmed": true,
        }),
        execution_context_with_network(
            "user_login_link_dispatch",
            "caller_login_link_dispatch",
            TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("onboard must succeed before login-link test");
    assert_eq!(
        onboard_result["enrolled"],
        json!(true),
        "must be enrolled before testing login link"
    );

    // ── Step 2: Invoke account_login_link with confirmed=true ─────────────────
    let result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
        json!({ "confirmed": true }),
        execution_context_with_network(
            "user_login_link_dispatch",
            "caller_login_link_dispatch",
            TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("account_login_link dispatch must succeed");

    assert_eq!(
        result["minted"],
        json!(true),
        "minted must be true on success; error_code={:?}, message={:?}",
        result.get("error_code"),
        result.get("message"),
    );
    assert_eq!(
        result["account_id"],
        json!("acc123"),
        "account_id must match mock response"
    );
    // SECURITY: the one-time login URL is a code-bearing account-access
    // credential. It must NOT be returned on the model-visible surface (that
    // copies it into the LLM transcript and any downstream logging). The result
    // delivers an opaque out-of-band marker instead — mirroring profile_token.
    assert!(
        result.get("url").is_none(),
        "the login URL credential must not be returned on the model-visible surface; got: {result}"
    );
    assert_eq!(
        result["link_delivery"],
        json!("local_private_account_login_link_file"),
        "out-of-band delivery must be signaled by an opaque marker"
    );
    assert!(
        !result.to_string().contains("testcode123"),
        "the one-time login code must never appear anywhere in the model-visible result"
    );

    // The URL is delivered out-of-band: persisted to a 0600 private
    // `account_login_link.url` file in the caller scope's local state dir for the
    // local UI/CLI to open. Locate it under the base dir without recomputing the
    // exact scope path.
    let persisted = find_persisted_login_link(setup_base_dir().path())
        .expect("the login URL must be persisted to the private account-login-link file");
    assert!(
        persisted.contains("/account/login?code=testcode123"),
        "the persisted private file must hold the one-time login URL; got: {persisted}"
    );
}
