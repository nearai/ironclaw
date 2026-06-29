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

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes.
#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

// Imports and helpers are gated alongside the test functions so that building
// without a database feature produces zero unused-import warnings.
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
//
// Proves that `CredentialAccountService::update_status(.., Revoked)` persists
// the status transition so a subsequent `get_account` reads back `Revoked`.
// Does not touch the refresh sweep or the HTTP egress.

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

    // Step 1 — create a Configured account via the OAuth connect flow.
    // After this, bundle.egress.captured_count() == 1 (initial token exchange).
    let account = connect_google_account(&bundle, &scope, 0x11).await;
    assert_eq!(
        account.status,
        CredentialAccountStatus::Configured,
        "freshly connected account must be Configured"
    );
    let account_id = account.id;

    // Step 2 — mark the account Revoked directly (no sweep required).
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

    // Step 3 — durable read-back: proves the mutation committed to the store,
    // not merely returned from update_status in-memory.
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

    // Step 1 — run the OAuth connect flow to create a Google credential account.
    // The default egress returns 200 with an access + refresh token body.
    // After this, egress.captured_count() == 1.
    let account = connect_google_account(&bundle, &scope, 0x22).await;
    let account_id = account.id;

    // Step 2 — queue an invalid_grant error response for the next egress call
    // (the sweep's refresh request).  The default 200 body stays in place for
    // any calls after that, but the sweep only makes one refresh call per
    // candidate, so only this one error is needed.
    bundle.egress.push_response(
        400,
        serde_json::json!({"error": "invalid_grant"})
            .to_string()
            .into_bytes(),
    );

    // Step 3 — freeze the clock 3 days ahead so the account (just created,
    // updated_at ≈ now) appears idle past the 2-day threshold.
    let frozen_now = Utc::now() + Duration::days(3);

    // Step 4 — run the sweep.  The queued 400 invalid_grant response causes
    // `HostOAuthProviderClient::refresh_token` to return
    // `AuthProductError::InvalidGrant`, which `refresh_account` maps to
    // `report_terminal_refresh_status(.., Revoked)`.
    bundle
        .sweep_for_refresh(
            vec![account],
            CredentialRefreshSettings::enabled(),
            frozen_now,
        )
        .await;

    // Step 5 — egress must have captured exactly 2 calls: the initial token
    // exchange (200) and the sweep refresh attempt (400 invalid_grant).
    assert_eq!(
        bundle.egress.captured_count(),
        2,
        "sweep must trigger exactly one refresh HTTP call \
         (total: initial exchange + sweep refresh attempt)"
    );

    // Step 6 — durable read-back: proves the invalid_grant caused the Revoked
    // status to be committed to the store.
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
//
// Proves that `Revoked` in arm (b) is caused by the `invalid_grant` error, not
// by the sweep machinery itself.  The same flow with a normal 200 egress must
// leave the account `Configured` (after a successful token rotation).

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

    // Step 1 — connect (egress call 1: 200 with access + refresh token).
    let account = connect_google_account(&bundle, &scope, 0x33).await;
    let account_id = account.id;

    // Step 2 — sweep with a frozen clock 3 days ahead (account appears idle).
    // No error response is queued; the default 200 egress is used throughout.
    let frozen_now = Utc::now() + Duration::days(3);
    bundle
        .sweep_for_refresh(
            vec![account],
            CredentialRefreshSettings::enabled(),
            frozen_now,
        )
        .await;

    // Step 3 — the sweep must have made exactly 2 egress calls (connect + refresh).
    assert_eq!(
        bundle.egress.captured_count(),
        2,
        "negative-guard sweep must trigger one refresh call (total: connect + refresh)"
    );

    // Step 4 — the account must NOT be Revoked after a successful refresh.
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
//
// Drives `ScriptedOAuthTokenEgress::execute` directly (no OAuth flow, no DB,
// no network) to verify the queued per-call override FIFO and the
// constructor-default fallback path, plus the `captured_count` accessor.
// Not gated on any database feature — it exercises only the scripted egress
// type itself.

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

    // Default response: 400 with `{"error":"invalid_grant"}`.
    let egress = ScriptedOAuthTokenEgress::with_error_response(400, "invalid_grant");

    // Queue two per-call overrides (FIFO: body_a first, body_b second).
    let body_a = b"queued-response-A".to_vec();
    let body_b = b"queued-response-B".to_vec();
    egress.push_response(200, body_a.clone());
    egress.push_response(500, body_b.clone());

    // Build a minimal dummy request.  `ScriptedOAuthTokenEgress::execute`
    // only reads `request.body.len()` and records the full request; the other
    // fields are unused by the scripted impl.
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

    // Call 1: first queued override consumed (FIFO).
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

    // Call 2: second queued override consumed (FIFO).
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

    // Call 3: queue exhausted — falls back to the constructor default.
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

    // All three execute calls must be captured.
    assert_eq!(
        egress.captured_count(),
        3,
        "all three execute calls must be captured by the scripted egress"
    );
}
