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

use chrono::{DateTime, Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowKind, AuthFlowStatus,
    AuthProductScope, AuthProviderId, AuthSurface, AuthorizationCodeHash, CredentialAccountLabel,
    CredentialAccountListRequest, CredentialAccountLookupRequest, NewAuthFlow,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope,
};
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use ironclaw_reborn_composition::{
    RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    test_support::build_oauth_product_auth_for_test,
};
use secrecy::SecretString;

/// Build a 64-character hex string from a repeated byte value.
fn hex64(fill: u8) -> String {
    format!("{fill:02x}").repeat(32)
}

fn test_scope() -> AuthProductScope {
    let resource =
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .expect("local_default scope must build");
    AuthProductScope::new(resource, AuthSurface::Callback)
}

/// A `NewAuthFlow` for the connect-flow tests; only identity hashes and the
/// expiry vary between scenarios.
fn new_flow_request(
    scope: &AuthProductScope,
    provider: &AuthProviderId,
    state_hash: &OpaqueStateHash,
    pkce_hash: &PkceVerifierHash,
    expires_at: DateTime<Utc>,
) -> NewAuthFlow {
    NewAuthFlow {
        id: None,
        scope: scope.clone(),
        kind: AuthFlowKind::IntegrationCredential,
        provider: provider.clone(),
        challenge: AuthChallenge::OAuthUrl {
            authorization_url: OAuthAuthorizationUrl::new(
                "https://accounts.example.com/o/oauth2/auth",
            )
            .unwrap(),
            expires_at,
        },
        continuation: AuthContinuationRef::SetupOnly,
        update_binding: None,
        opaque_state_hash: Some(state_hash.clone()),
        pkce_verifier_hash: Some(pkce_hash.clone()),
        expires_at,
    }
}

/// An `Authorized` provider-callback request matching `new_flow_request`'s
/// hashes — the "user completed the provider consent page" leg.
fn authorized_callback_request(
    scope: &AuthProductScope,
    flow_id: AuthFlowId,
    provider: &AuthProviderId,
    state_hash: &OpaqueStateHash,
    pkce_hash: &PkceVerifierHash,
    code_hash: &AuthorizationCodeHash,
    label: &str,
) -> RebornOAuthCallbackRequest {
    RebornOAuthCallbackRequest {
        scope: scope.clone(),
        flow_id,
        opaque_state_hash: state_hash.clone(),
        outcome: RebornOAuthCallbackOutcome::Authorized {
            provider_request: OAuthProviderCallbackRequest {
                provider: provider.clone(),
                account_label: CredentialAccountLabel::new(label).unwrap(),
                authorization_code: OAuthAuthorizationCode::new(SecretString::from(format!(
                    "auth-code-{label}"
                )))
                .unwrap(),
                authorization_code_hash: code_hash.clone(),
                pkce_verifier: PkceVerifierSecret::new(SecretString::from(format!(
                    "pkce-verifier-{label}"
                )))
                .unwrap(),
                pkce_verifier_hash: pkce_hash.clone(),
                scopes: vec![ProviderScope::new("test.readonly").unwrap()],
            },
        },
    }
}

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

/// T4 of the #6105 lifecycle transitions (issues #2858/#2534/#6043 shape): a
/// callback that lands AFTER the flow lapsed (the user abandoned or lost the
/// provider tab) must be rejected terminally — and a RETRIED connect with a
/// fresh flow must then succeed cleanly. The expired first attempt must leave
/// no half-connected state behind: its record reads back terminal `Expired`,
/// no token exchange crossed the egress for it, and after the retry exactly
/// one credential account exists.
#[tokio::test]
async fn expired_flow_callback_rejected_then_fresh_flow_retry_succeeds() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();

    let state_hash = OpaqueStateHash::new(hex64(0x11)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x22)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x33)).unwrap();

    // The flow lapsed before the callback arrived.
    let expired_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &state_hash,
            &pkce_hash,
            Utc::now() - Duration::seconds(10),
        ))
        .await
        .expect("create_flow must accept a flow that will read back as lapsed");

    let error = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            expired_flow.id,
            &provider,
            &state_hash,
            &pkce_hash,
            &code_hash,
            "Late Callback",
        ))
        .await
        .expect_err("a callback for a lapsed flow must be rejected");
    assert_eq!(
        error.code,
        AuthErrorCode::UnknownOrExpiredFlow,
        "lapsed-flow callback must surface as UnknownOrExpiredFlow"
    );

    // Durable evidence of a clean rejection: the record is terminal Expired
    // (not half-claimed), and the token exchange never ran.
    let record = bundle
        .services
        .flow_manager()
        .get_flow(&scope, expired_flow.id)
        .await
        .expect("get_flow must not error")
        .expect("the expired flow record must remain readable");
    assert_eq!(
        record.status,
        AuthFlowStatus::Expired,
        "the lapsed flow must be marked terminal Expired, not left pending"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        0,
        "no token exchange may run for a lapsed-flow callback"
    );

    // Retry: a fresh flow (new state/PKCE — a new grant) for the same
    // provider and scope. An expired predecessor must not fence it out.
    let retry_state = OpaqueStateHash::new(hex64(0x44)).unwrap();
    let retry_pkce = PkceVerifierHash::new(hex64(0x55)).unwrap();
    let retry_code = AuthorizationCodeHash::new(hex64(0x66)).unwrap();
    let retry_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &retry_state,
            &retry_pkce,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("a retried create_flow must succeed after an expired predecessor");
    let response = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            retry_flow.id,
            &provider,
            &retry_state,
            &retry_pkce,
            &retry_code,
            "Retry Account",
        ))
        .await
        .expect("the retried callback must succeed; an expired flow must not wedge the retry");
    let account_id = response
        .credential_account_id
        .expect("the retried callback must mint a credential account");

    // Exactly ONE account and ONE token exchange: the failed first attempt
    // contributed nothing.
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(
            scope.clone(),
            provider.clone(),
        ))
        .await
        .expect("list_accounts must not error after the retry");
    assert_eq!(
        page.accounts.len(),
        1,
        "only the retry's account may exist; a lapsed flow must not leave a half-connected account"
    );
    assert_eq!(
        page.accounts[0].id, account_id,
        "the surviving account is the retry's account"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "exactly the retry's token exchange may cross the egress"
    );
}

/// T4 of the #6105 lifecycle transitions, replay arm (issue #2858 shape): a
/// callback REPLAYED after the flow already completed (browser back-button /
/// duplicated redirect) is IDEMPOTENT on the durable services: it returns the
/// original completed outcome — same credential account — without re-running
/// the token exchange or minting a second account, and a subsequent fresh
/// connect (reconnect epoch) still succeeds.
///
/// NOT a fake-vs-durable divergence (both impls agree at every
/// `AuthFlowManager` method — see `ironclaw_auth::conformance`): the
/// replay-safety split is between SEAMS. `claim_oauth_callback` is
/// idempotent on terminal flows in both impls, and `handle_oauth_callback`
/// (this wrapper, the hosted unauthenticated callback route's entry)
/// short-circuits at that claim — while trait-level
/// `complete_oauth_callback` stays fail-closed (`FlowAlreadyTerminal`) in
/// both. This test pins the wrapper's replay-idempotent shape.
#[tokio::test]
async fn replayed_callback_is_idempotent_then_fresh_flow_reconnects() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();

    let state_hash = OpaqueStateHash::new(hex64(0x71)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x72)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x73)).unwrap();

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
    let first = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            flow.id,
            &provider,
            &state_hash,
            &pkce_hash,
            &code_hash,
            "First Grant",
        ))
        .await
        .expect("the first callback must complete the flow");
    let first_account = first
        .credential_account_id
        .expect("the first callback must mint a credential account");
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "the first callback runs exactly one token exchange"
    );

    // Replay the identical callback (browser back-button / duplicated
    // redirect). It must return the ORIGINAL completed outcome — not error,
    // not a half-processed or duplicated grant.
    let replay = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            flow.id,
            &provider,
            &state_hash,
            &pkce_hash,
            &code_hash,
            "First Grant",
        ))
        .await
        .expect("a replayed callback for a completed flow must be idempotent, not an error");
    assert_eq!(
        replay.credential_account_id,
        Some(first_account),
        "the replayed callback must return the original grant's credential account"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "a replayed callback must not re-run the token exchange"
    );
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(
            scope.clone(),
            provider.clone(),
        ))
        .await
        .expect("list_accounts must not error after the replay");
    assert_eq!(
        page.accounts.len(),
        1,
        "a replayed callback must not mint a second credential account"
    );

    // Reconnect: a fresh flow must still succeed after the idempotent replay
    // (which returned the ORIGINAL completed outcome — no rejection occurred).
    let reconnect_state = OpaqueStateHash::new(hex64(0x81)).unwrap();
    let reconnect_pkce = PkceVerifierHash::new(hex64(0x82)).unwrap();
    let reconnect_code = AuthorizationCodeHash::new(hex64(0x83)).unwrap();
    let reconnect_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &reconnect_state,
            &reconnect_pkce,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("a reconnect create_flow must succeed after a replayed predecessor");
    let reconnect = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            reconnect_flow.id,
            &provider,
            &reconnect_state,
            &reconnect_pkce,
            &reconnect_code,
            "Reconnect Grant",
        ))
        .await
        .expect("the reconnect callback must succeed; a replayed flow must not wedge reconnects");
    let reconnect_account = reconnect
        .credential_account_id
        .expect("the reconnect callback must mint a credential account");
    assert_ne!(
        reconnect_account, first_account,
        "a fresh reconnect flow must mint a DISTINCT credential account, not reuse the original"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        2,
        "the reconnect must run its own token exchange (first callback's plus one)"
    );
    let listed = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(scope, provider))
        .await
        .expect("list_accounts must not error after the reconnect");
    assert_eq!(
        listed.accounts.len(),
        2,
        "exactly the original and the reconnect accounts must exist"
    );
    for expected in [&first_account, &reconnect_account] {
        assert!(
            listed
                .accounts
                .iter()
                .any(|account| &account.id == expected),
            "account {expected:?} must be listed after the reconnect"
        );
    }
}
