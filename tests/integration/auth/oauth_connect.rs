//! Reborn integration-test framework — slice 7: OAuth connect-flow.
//!
//! Drives a real OAuth connect flow through the Reborn product-auth boundary:
//! `create_flow` → `handle_oauth_callback` → assert `CredentialAccount`
//! persisted and readable.  The token-exchange HTTP is captured by a
//! `ScriptedOAuthTokenEgress` (no real network); all other stores (flow +
//! account persistence) are real `FilesystemAuthProductServices<InMemoryBackend>`.
//!
//! This proves design-spec §3.8 coverage: real stores, mock only the OAuth HTTP
//! seam at the `RuntimeHttpEgress` boundary.
//!
//! The abandoned/denied/expired/replayed POPUP journeys over the same seam
//! live next door in `oauth_popup_journeys.rs`.

#[path = "common.rs"]
mod common;

use chrono::{Duration, Utc};
use common::{authorized_callback_request, hex64, new_flow_request, test_scope};
use ironclaw_auth::{
    AuthErrorCode, AuthFlowId, AuthProviderId, AuthorizationCodeHash, CredentialAccountListRequest,
    CredentialAccountLookupRequest, OpaqueStateHash, PkceVerifierHash,
};
use ironclaw_reborn_composition::test_support::build_oauth_product_auth_for_test;

/// Core slice-7 scenario: a real OAuth connect flow produces a persisted
/// `CredentialAccount` that reads back correctly, and exactly one
/// token-exchange HTTP call was made to the scripted egress.
#[tokio::test]
async fn oauth_connect_flow_persists_credential_account() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();

    // Stable hash values shared across flow creation and callback claim.
    let state_hash = OpaqueStateHash::new(hex64(0xaa)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0xbb)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0xcc)).unwrap();

    let flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &state_hash,
            &pkce_hash,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("create_flow must succeed");

    // Drives claim → token exchange → complete. The scripted egress returns a
    // fixed access-token JSON body; no real network call is made.
    let response = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            flow.id,
            &provider,
            &state_hash,
            &pkce_hash,
            &code_hash,
            "Test Account",
        ))
        .await
        .expect("handle_oauth_callback must succeed");

    let account_id = response
        .credential_account_id
        .expect("completed callback must carry a credential_account_id");

    let account = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(scope, account_id))
        .await
        .expect("get_account must not error")
        .expect("credential account must be persisted after a successful OAuth callback");

    assert_eq!(
        account.id, account_id,
        "account id matches the callback response"
    );
    assert_eq!(
        account.provider, provider,
        "account provider matches the flow provider"
    );

    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "exactly one token-exchange HTTP call must be captured by the scripted egress"
    );

    // Must use authorization_code, not the refresh grant — proves the right
    // OAuth flow crossed the egress.
    let grant_types = bundle.egress.captured_grant_types();
    assert_eq!(
        grant_types.first().map(String::as_str),
        Some("authorization_code"),
        "connect-flow token exchange must use the authorization_code grant; grant_types: {grant_types:?}"
    );
}

/// Cross-implementation conformance: the durable `FilesystemAuthProductServices`
/// must satisfy the same observable OAuth-callback state machine
/// (`ironclaw_auth::test_support::conformance`) as the in-memory fake most consumer tests
/// run against; the fake's invocation lives in
/// `crates/ironclaw_auth/tests/auth_product_contract/oauth_flow_contract.rs`.
/// The suite drives `AuthFlowManager` directly with pre-exchanged outcomes,
/// so no token-exchange egress is involved — the exchange leg is covered by
/// the surrounding tests in this file.
#[tokio::test]
async fn durable_flow_manager_satisfies_shared_oauth_flow_conformance() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();
    ironclaw_auth::test_support::conformance::assert_auth_flow_callback_conformance(
        bundle.services.flow_manager().as_ref(),
        &scope,
        &provider,
    )
    .await;
}

/// Guard test: attempting an OAuth callback for a non-existent flow must fail
/// with `UnknownOrExpiredFlow`.  No credential account must be created, and no
/// token-exchange call should be made.
///
/// Both guarantees are verified: `captured_count()` asserts no token-exchange
/// HTTP call was made; `list_accounts` asserts no credential account was
/// persisted to the durable store.
#[tokio::test]
async fn oauth_callback_without_prior_flow_fails() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let state_hash = OpaqueStateHash::new(hex64(0xdd)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0xee)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0xff)).unwrap();

    let error = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            AuthFlowId::new(), // no flow was created for this id
            &AuthProviderId::new("test-oauth-provider").unwrap(),
            &state_hash,
            &pkce_hash,
            &code_hash,
            "Guard Account",
        ))
        .await
        .expect_err("callback with no prior flow must return an error");

    assert_eq!(
        error.code,
        AuthErrorCode::UnknownOrExpiredFlow,
        "missing flow must surface as UnknownOrExpiredFlow"
    );

    // The claim step fails before any token-exchange — egress must be clean.
    assert_eq!(
        bundle.egress.captured_count(),
        0,
        "no token-exchange call should be made when the flow is missing"
    );

    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(
            scope,
            AuthProviderId::new("test-oauth-provider").unwrap(),
        ))
        .await
        .expect("list_accounts must not error after a failed callback");
    assert!(
        page.accounts.is_empty(),
        "no credential account must be created when the flow is missing; got {} accounts",
        page.accounts.len()
    );
}
