//! Reborn integration-test framework — auth/credential-failure coverage.
//!
//! Two arms:
//!
//! **(a) `revoked_account_reads_back_revoked`** — pure-store: create a
//! `Configured` credential account via the standard OAuth connect flow, call
//! `CredentialAccountService::update_status(.., Revoked)`, and assert the
//! durable read-back carries `Revoked`.  This arm exercises the status-mutation
//! path without touching the refresh sweep.
//!
//! **(b) `invalid_grant_sweep_marks_account_revoked`** — end-to-end refresh
//! failure: an idle Google OAuth account receives a scripted `invalid_grant`
//! 400 response from the token endpoint during a credential-refresh sweep, and
//! the durable record is subsequently `Revoked`.  The sweep path is
//! `sweep_once` → `ProviderBackedCredentialAccountService::refresh_account` →
//! `HostOAuthProviderClient::refresh_token` → scripted HTTP egress.
//!
//! **(negative guard) `normal_sweep_does_not_mark_account_revoked`** — the
//! same sweep flow with a normal `200` egress must leave the account status as
//! `Configured`, proving that `Revoked` in arm (b) is caused by the
//! `invalid_grant` error, not by the sweep machinery itself.
//!
//! **Deferred — live-401 re-auth arm**: reactive re-auth after a credentialed
//! capability backend returns HTTP 401 requires a credentialed capability
//! backend stub and is out of scope here.  Track as a follow-up once a
//! `CapabilityBackend` test double is available.
//!
//! All three test functions are gated on
//! `any(feature = "libsql", feature = "postgres")` (the same gate as the
//! `credential_refresh_worker` that powers arm b) so the file compiles and
//! produces zero tests when neither database feature is active.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

#[cfg(any(feature = "libsql", feature = "postgres"))]
use chrono::{Duration, Utc};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_auth::{
    AuthProductScope, AuthSurface, CredentialAccountLookupRequest, CredentialAccountStatus,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_reborn_composition::{
    CredentialRefreshSettings, test_support::build_google_oauth_product_auth_for_test,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use reborn_support::oauth_flow::connect_google_account;

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn test_scope() -> AuthProductScope {
    let resource = ResourceScope::local_default(
        UserId::new("auth-failure-test-user").unwrap(),
        InvocationId::new(),
    )
    .expect("local_default scope must build");
    AuthProductScope::new(resource, AuthSurface::Callback)
}

// ─── arm a: pure-store revoke ─────────────────────────────────────────────────

/// Create a `Configured` credential account, mark it `Revoked` via
/// `update_status`, and verify the durable read-back carries `Revoked`.
///
/// `Revoked` is a terminal status: no further OAuth flow is expected.
/// The assertion on the read-back proves the status change committed to the
/// durable `FilesystemAuthProductServices<InMemoryBackend>` store and was not
/// merely returned in-memory by `update_status`.
#[cfg(any(feature = "libsql", feature = "postgres"))]
#[tokio::test]
async fn revoked_account_reads_back_revoked() {
    let bundle = build_google_oauth_product_auth_for_test();
    let scope = test_scope();

    let account = connect_google_account(&bundle, &scope, 0x11).await;
    assert_eq!(
        account.status,
        CredentialAccountStatus::Configured,
        "freshly connected account must be Configured"
    );
    let account_id = account.id;

    let updated = bundle
        .services
        .credential_account_service()
        .update_status(&scope, account_id, CredentialAccountStatus::Revoked)
        .await
        .expect("update_status must succeed");
    assert_eq!(
        updated.status,
        CredentialAccountStatus::Revoked,
        "update_status return value must carry Revoked"
    );

    // Durable read-back proves the mutation committed to the store, not
    // merely returned from update_status in-memory.
    let read_back = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .expect("get_account must not error after update_status")
        .expect("account must still be present after Revoked transition");
    assert_eq!(
        read_back.status,
        CredentialAccountStatus::Revoked,
        "durable read-back must carry Revoked after update_status; got {:?}",
        read_back.status
    );
}

// ─── arm b: invalid_grant sweep marks account Revoked ─────────────────────────
//
// Proves the end-to-end path:
//   sweep_once → select_idle_candidates → RebornProductAuthServices::refresh_credential_account
//     → ProviderBackedCredentialAccountService::refresh_account
//     → HostOAuthProviderClient::refresh_token (HTTP egress: 400 invalid_grant)
//     → AuthProductError::InvalidGrant
//     → report_terminal_refresh_status(.., Revoked)
//
// The egress mock uses the `push_response` mechanism: the bundle's default
// egress returns 200 (so the initial token exchange succeeds and stores a real
// refresh-secret handle), then a 400 `invalid_grant` response is queued for
// the sweep's refresh call.

/// An idle Google OAuth account that receives `{"error":"invalid_grant"}` from
/// the token endpoint during a credential-refresh sweep is persistently marked
/// `Revoked` by `refresh_account`.
///
/// Egress call count asserts:
///   1 = initial token exchange (connect flow, 200)
///   2 = sweep refresh attempt (queued 400 invalid_grant)
///
/// The account write-back is verified through the durable
/// `CredentialAccountService::get_account` path (not just the refresh report
/// return value), guarding against the "HTTP fired but account write dropped"
/// failure mode.
#[cfg(any(feature = "libsql", feature = "postgres"))]
#[tokio::test]
async fn invalid_grant_sweep_marks_account_revoked() {
    let bundle = build_google_oauth_product_auth_for_test();
    let scope = test_scope();

    // egress.captured_count() == 1 after this (initial token exchange).
    let account = connect_google_account(&bundle, &scope, 0x22).await;
    let account_id = account.id;

    // The sweep makes exactly one refresh call per candidate, so a single
    // queued invalid_grant response is enough; the constructor-default 200
    // body stays in place for any calls after that.
    bundle.egress.push_response(
        400,
        serde_json::json!({"error": "invalid_grant"})
            .to_string()
            .into_bytes(),
    );

    // Freeze the clock 3 days ahead so the account (just created,
    // updated_at ≈ now) appears idle past the 2-day threshold.
    let frozen_now = Utc::now() + Duration::days(3);

    bundle
        .sweep_for_refresh(
            vec![account],
            CredentialRefreshSettings::enabled(),
            frozen_now,
        )
        .await;

    assert_eq!(
        bundle.egress.captured_count(),
        2,
        "sweep must trigger exactly one refresh HTTP call \
         (total: initial exchange + sweep refresh attempt)"
    );

    // Durable read-back proves the invalid_grant error is what committed the
    // Revoked status, not just the sweep's in-memory return value.
    let post_sweep = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .expect("get_account must not error after sweep")
        .expect("account must still be present after sweep");
    assert_eq!(
        post_sweep.status,
        CredentialAccountStatus::Revoked,
        "account must be Revoked after an invalid_grant response from the token endpoint; \
         got {:?}",
        post_sweep.status
    );
}

// ─── negative guard ────────────────────────────────────────────────────────────

/// A credential-refresh sweep with a normal `200` egress MUST NOT mark the
/// account `Revoked`.
///
/// After a successful sweep the account is `Configured` (tokens rotated) and
/// the egress call count is 2 (initial exchange + refresh).
#[cfg(any(feature = "libsql", feature = "postgres"))]
#[tokio::test]
async fn normal_sweep_does_not_mark_account_revoked() {
    let bundle = build_google_oauth_product_auth_for_test();
    let scope = test_scope();

    let account = connect_google_account(&bundle, &scope, 0x33).await;
    let account_id = account.id;

    // No error response is queued; the default 200 egress is used throughout.
    let frozen_now = Utc::now() + Duration::days(3);
    bundle
        .sweep_for_refresh(
            vec![account],
            CredentialRefreshSettings::enabled(),
            frozen_now,
        )
        .await;

    assert_eq!(
        bundle.egress.captured_count(),
        2,
        "negative-guard sweep must trigger one refresh call (total: connect + refresh)"
    );

    let post_sweep = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .expect("get_account must not error after successful sweep")
        .expect("account must still be present after successful sweep");
    assert_ne!(
        post_sweep.status,
        CredentialAccountStatus::Revoked,
        "a successful refresh sweep must not mark the account Revoked; got {:?}",
        post_sweep.status
    );
    assert_eq!(
        post_sweep.status,
        CredentialAccountStatus::Configured,
        "account must remain Configured after a successful refresh sweep; got {:?}",
        post_sweep.status
    );
}

// ─── FIFO + default-fallback unit test ───────────────────────────────────────

/// [`ScriptedOAuthTokenEgress`] queued responses are consumed in FIFO order;
/// once the queue is exhausted, subsequent calls fall back to the constructor
/// default.
///
/// This test drives [`RuntimeHttpEgress::execute`] directly — no OAuth flow,
/// no real network — to verify the FIFO queue + default-fallback path and
/// the [`captured_count`] accessor in isolation, exercising the
/// `push_response` / `with_error_response` pairing described in the
/// `ScriptedOAuthTokenEgress` documentation.
#[tokio::test]
async fn scripted_oauth_token_egress_consumes_queued_responses_fifo_then_default() {
    use ironclaw_host_api::{
        CapabilityId, InvocationId, NetworkMethod, NetworkPolicy, ResourceScope, RuntimeHttpEgress,
        RuntimeHttpEgressRequest, RuntimeKind, UserId,
    };
    use ironclaw_reborn_composition::test_support::ScriptedOAuthTokenEgress;

    let egress = ScriptedOAuthTokenEgress::with_error_response(400, "invalid_grant");

    let body_a = b"queued-response-A".to_vec();
    let body_b = b"queued-response-B".to_vec();
    egress.push_response(200, body_a.clone());
    egress.push_response(500, body_b.clone());

    // `ScriptedOAuthTokenEgress::execute` only reads `request.body.len()` and
    // records the full request; the other fields are unused by the scripted
    // impl, so this dummy request leaves them empty/default.
    let dummy_request = || RuntimeHttpEgressRequest {
        runtime: RuntimeKind::Wasm,
        scope: ResourceScope::local_default(
            UserId::new("fifo-test-user").unwrap(),
            InvocationId::new(),
        )
        .expect("local_default scope must build"),
        capability_id: CapabilityId::new("builtin.test").unwrap(),
        method: NetworkMethod::Post,
        url: "https://oauth.test.example.com/token".to_string(),
        headers: vec![],
        body: vec![],
        network_policy: NetworkPolicy::default(),
        credential_injections: vec![],
        response_body_limit: None,
        save_body_to: None,
        timeout_ms: None,
    };

    let resp1 = egress
        .execute(dummy_request())
        .await
        .expect("execute call 1 must not error");
    assert_eq!(
        resp1.status, 200,
        "call 1 must return the first queued status (200)"
    );
    assert_eq!(
        resp1.body, body_a,
        "call 1 must return the first queued body"
    );

    let resp2 = egress
        .execute(dummy_request())
        .await
        .expect("execute call 2 must not error");
    assert_eq!(
        resp2.status, 500,
        "call 2 must return the second queued status (500)"
    );
    assert_eq!(
        resp2.body, body_b,
        "call 2 must return the second queued body"
    );

    let resp3 = egress
        .execute(dummy_request())
        .await
        .expect("execute call 3 must not error");
    assert_eq!(
        resp3.status, 400,
        "call 3 must fall back to the default error status (400)"
    );
    let resp3_body = String::from_utf8_lossy(&resp3.body);
    assert!(
        resp3_body.contains("invalid_grant"),
        "call 3 must fall back to the default error body containing 'invalid_grant'; got: {resp3_body}"
    );

    assert_eq!(
        egress.captured_count(),
        3,
        "all three execute calls must be captured by the scripted egress"
    );
}
