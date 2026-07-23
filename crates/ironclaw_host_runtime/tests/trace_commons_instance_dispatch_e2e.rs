//! End-to-end test for the **instance-enrollment** path through the host-runtime
//! capability-dispatch layer.
//!
//! This is a SEPARATE test binary from `trace_commons_dispatch_e2e.rs` on
//! purpose: it provisions the process-global instance policy (scope `None`),
//! which would otherwise bleed into the personal-invite suite whose tests rely
//! on per-user not-enrolled defaults. A distinct binary = a fresh process = a
//! private `IRONCLAW_BASE_DIR`, so the two suites cannot interfere.
//!
//! It pins the fix for the CodeRabbit #5 finding at the layer it manifests: an
//! instance-only-enrolled user (admin-provisioned instance policy, no personal
//! invite) must pass the dispatch enrollment gate and mint under the shared
//! instance device key with a per-user pseudonymous subject.

use std::sync::{Arc, Mutex};

use ironclaw_host_runtime::{
    TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID, TRACE_COMMONS_STATUS_CAPABILITY_ID,
};
use ironclaw_traces::contribution::trace_contribution_dir_for_scope;
use ironclaw_traces::onboarding::{OnboardConsents, onboard_at_dir};
use serde_json::json;

#[path = "support/trace_commons_dispatch.rs"]
mod tc_support;
use tc_support::*;

/// Instance-only enrollment through dispatch: a user who never ran the personal
/// onboarding flow must still be treated as enrolled (via the admin-provisioned
/// instance policy) by `dispatch_status` and `dispatch_account_login_link`, and
/// the mint must attribute to that user's pseudonymous subject.
#[tokio::test]
async fn instance_only_user_passes_dispatch_gate_and_mints_login_link() {
    let _base_dir = setup_base_dir();

    let claim_jwt = test_jwt_eddsa("instance-e2e-key");
    let claim_jwt_for_mock = claim_jwt.clone();
    let login_bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let login_cap = login_bodies.clone();

    // Mock with the three routes the instance flow touches:
    //   /v1/onboard             — instance provisioning POST (setup)
    //   /v1/trace-upload-claim  — device-key bearer issuer (reqwest path)
    //   /v1/account/login-links — the endpoint under test (captures subject)
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock server binds");
    let addr = listener.local_addr().expect("mock server local addr");

    let app = {
        let addr_for_onboard = addr;
        axum::Router::new()
            .route(
                "/v1/onboard",
                axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                    let port = addr_for_onboard.port();
                    async move {
                        let pubkey_b64 = body["device_public_key"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        let device_key_id = derive_device_key_id(&pubkey_b64)
                            .unwrap_or_else(|| "sha256:unknown".to_string());
                        axum::Json(json!({
                            "schema_version": "trace_commons.onboard_response.v1",
                            "tenant_id": "tenant-instance",
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
                    let jwt = claim_jwt_for_mock.clone();
                    async move {
                        axum::Json(json!({
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
                    let login_cap = login_cap.clone();
                    async move {
                        login_cap.lock().unwrap().push(b);
                        axum::Json(json!({
                            "account_id": "instanceacc",
                            "url": "/account/login?code=instancecode456"
                        }))
                    }
                }),
            )
    };
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    // ── Provision INSTANCE enrollment (scope None), NOT a personal invite ─────
    // The test user below never onboards personally, so its personal policy stays
    // absent — only the shared instance policy is enabled.
    let invite_url = format!("http://127.0.0.1:{}/onboard#INSTANCEE2E", addr.port());
    onboard_at_dir(
        &trace_contribution_dir_for_scope(None),
        &invite_url,
        OnboardConsents {
            include_message_text: false,
            include_tool_payloads: false,
        },
    )
    .await
    .expect("instance onboarding must succeed");

    let rt = runtime();

    // ── status: an instance-only user must report enrolled ────────────────────
    let status = invoke_with_context(
        &rt,
        TRACE_COMMONS_STATUS_CAPABILITY_ID,
        json!({}),
        execution_context_with_network(
            "instance_only_user",
            "caller_instance_only",
            TRACE_COMMONS_STATUS_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("status dispatch must succeed");
    assert_eq!(
        status["enrolled"],
        json!(true),
        "instance-only user must be reported as enrolled; got {status}"
    );

    // ── account_login_link: must pass the gate and mint (was falsely rejected) ─
    let result = invoke_with_context(
        &rt,
        TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
        json!({ "confirmed": true }),
        execution_context_with_network(
            "instance_only_user",
            "caller_instance_only",
            TRACE_COMMONS_ACCOUNT_LOGIN_LINK_CAPABILITY_ID,
            allow_all_network_policy(),
        ),
    )
    .await
    .expect("account_login_link dispatch must succeed");

    assert_eq!(
        result["minted"],
        json!(true),
        "instance-only user must mint a login link; error_code={:?}, message={:?}",
        result.get("error_code"),
        result.get("message"),
    );
    // Same out-of-band credential handling as the personal path.
    assert!(
        result.get("url").is_none(),
        "the login URL credential must not be returned on the model-visible surface; got {result}"
    );
    assert_eq!(
        result["link_delivery"],
        json!("local_private_account_login_link_file")
    );

    // Instance attribution: the login-links POST carries a per-user pseudonymous
    // subject (personal-invite enrollment omits it). A non-empty subject proves
    // the mint resolved the instance enrollment rather than the absent personal
    // policy — the crux of the fix.
    let bodies = login_bodies.lock().unwrap();
    assert_eq!(bodies.len(), 1, "exactly one login-links POST");
    assert!(
        bodies[0]["subject"].as_str().is_some_and(|s| !s.is_empty()),
        "instance mint must POST a non-empty per-user subject; got {}",
        bodies[0]
    );

    // The one-time URL is delivered out-of-band to a private 0600 file.
    let persisted = find_persisted_login_link(setup_base_dir().path())
        .expect("the login URL must be persisted to the private account-login-link file");
    assert!(
        persisted.contains("/account/login?code=instancecode456"),
        "persisted private file must hold the one-time login URL; got: {persisted}"
    );
}
