//! Reborn integration-test framework — slice 8: OAuth credential-refresh sweep.
//!
//! Proves the proactive keepalive sweep refreshes an idle Google OAuth account
//! with the token-refresh HTTP scripted through `ScriptedOAuthTokenEgress` (no
//! real network) and real credential stores on a `FilesystemAuthProductServices
//! <InMemoryBackend>` composite.
//!
//! Clock injection (`now: DateTime<Utc>` parameter on `sweep_once`) lets a test
//! freeze time 3 days ahead so a just-created account appears idle without an
//! actual wait.  Design spec §9 build-order, step 8.

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../../support/mod.rs"]
mod support;

use chrono::{Duration, Utc};
use ironclaw_auth::{AuthProductScope, AuthSurface, CredentialAccountLookupRequest};
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use ironclaw_reborn_composition::{
    CredentialRefreshSettings, test_support::build_google_oauth_product_auth_for_test,
};
use reborn_support::oauth_flow::connect_google_account;

fn test_scope() -> AuthProductScope {
    let resource =
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .expect("local_default scope must build");
    AuthProductScope::new(resource, AuthSurface::Callback)
}

/// Positive test: a sweep with a frozen clock 3 days ahead (past the 2-day
/// idle threshold) triggers a token-refresh HTTP call for the idle account AND
/// commits the rotated credential to the durable store.
///
/// Egress count alone only proves the HTTP call *fired*; it would still pass
/// if the write was silently dropped. To close that gap, re-read the account
/// and assert the persisted access-token handle was rewritten to the
/// refresh-path handle (`…-oauth-refresh-access-<account_id>`, produced only
/// by `store_refreshed_tokens`'s write-back) rather than the original
/// connect-exchange handle — which also transitively proves the refreshed
/// material was persisted to the secret store.
#[tokio::test]
async fn credential_refresh_sweep_refreshes_idle_google_account() {
    let bundle = build_google_oauth_product_auth_for_test();
    let scope = test_scope();

    // Capture the connect-exchange access handle before `account` moves into
    // the sweep candidate list, so the post-sweep read-back can prove it was
    // rewritten by the refresh.
    let account = connect_google_account(&bundle, &scope, 0xaa).await;
    let account_id = account.id;
    let connect_access_handle = account
        .access_secret
        .clone()
        .expect("connect flow must persist an access-token handle");

    // Freeze the clock 3 days ahead: the account was just created (updated_at
    // ≈ Utc::now()), so idle_cutoff = frozen_now − 2 days is still 1 day ahead
    // of creation, making the account idle.
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
        "sweep must trigger exactly one refresh HTTP call for the idle account \
         (total egress count: initial exchange + refresh)"
    );

    // Must use the refresh_token grant, not a second authorization_code
    // exchange — proves the refresh path, not a re-connect.
    let grant_types = bundle.egress.captured_grant_types();
    assert_eq!(
        grant_types.get(1).map(String::as_str),
        Some("refresh_token"),
        "sweep token exchange must use the refresh_token grant; grant_types: {grant_types:?}"
    );

    // Re-read the account through the durable account service to prove the
    // refresh COMMITTED the rotated credential (guards the "HTTP fired but the
    // account write was dropped" failure mode) — this reads the REAL persisted
    // record, not the in-test `account` variable.
    let refreshed = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .expect("get_account must not error after the refresh sweep")
        .expect("account must still be persisted after a successful refresh");
    let refreshed_access_handle = refreshed
        .access_secret
        .expect("refreshed account must still carry an access-token handle");

    assert_ne!(
        refreshed_access_handle.as_str(),
        connect_access_handle.as_str(),
        "refresh must rewrite the persisted access-token handle; the original \
         connect-exchange handle still being present means the account write was \
         dropped"
    );
    assert!(
        refreshed_access_handle
            .as_str()
            .contains("oauth-refresh-access"),
        "persisted access-token handle must be the refresh-path handle \
         (`…-oauth-refresh-access-<account_id>`), proving the refresh write-back \
         committed; got: {}",
        refreshed_access_handle.as_str()
    );
}

/// Guard test: a sweep with the real clock does NOT refresh a freshly-created
/// account that is still within the 2-day idle threshold.
#[tokio::test]
async fn credential_refresh_sweep_skips_fresh_google_account() {
    let bundle = build_google_oauth_product_auth_for_test();
    let scope = test_scope();

    let account = connect_google_account(&bundle, &scope, 0xbb).await;

    // Sweep with Utc::now() as the clock: the account was just created, so
    // updated_at is effectively Utc::now(); idle_cutoff = now − 2 days is
    // 2 days ago, which is BEFORE updated_at → account is NOT idle.
    bundle
        .sweep_for_refresh(
            vec![account],
            CredentialRefreshSettings::enabled(),
            Utc::now(),
        )
        .await;

    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "sweep must not refresh a freshly-created account that is still within \
         the idle threshold (egress count must stay at 1, the initial exchange)"
    );
}
