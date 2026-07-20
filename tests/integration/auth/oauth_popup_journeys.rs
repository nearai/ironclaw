//! The OAuth connect-POPUP user journeys over the real product-auth boundary
//! (same seam as `oauth_connect.rs`: real flow + account stores, token
//! exchange captured by `ScriptedOAuthTokenEgress`):
//!
//! - the user abandons the popup and the flow lapses — a LATE callback is
//!   rejected terminally and a fresh retry connects cleanly;
//! - the browser REPLAYS a completed callback (back-button / duplicated
//!   redirect) — idempotent, no second account, and a later reconnect works;
//! - the user CLOSES the popup and clicks Connect again — creating the
//!   reopened flow supersedes the abandoned one at the `create_flow` seam,
//!   and the abandoned tab's late callback dies at claim as terminal;
//! - the user DENIES consent on the provider page — the flow terminalizes as
//!   as `ProviderDenied` with no exchange and no account, and an immediate fresh Connect
//!   succeeds.

#[path = "common.rs"]
mod common;

use chrono::{Duration, Utc};
use common::{authorized_callback_request, hex64, new_flow_request, test_scope};
use ironclaw_auth::{
    AuthErrorCode, AuthFlowOutcome, AuthFlowState, AuthProviderId, AuthorizationCodeHash,
    CredentialAccountListRequest, OpaqueStateHash, PkceVerifierHash,
};
use ironclaw_reborn_composition::{
    RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    test_support::build_oauth_product_auth_for_test,
};

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
        record.state,
        AuthFlowState::Resolved(AuthFlowOutcome::Expired),
        "the lapsed flow must be resolved as Expired, not left open"
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

/// The connect-popup journey: the user opens the OAuth popup (flow A), closes
/// it without authorizing, and clicks Connect again (flow B). Creating flow B
/// supersedes the abandoned attempt at the `create_flow` seam itself — A reads
/// back resolved as aborted — and completing B mints exactly one credential
/// account. The abandoned tab's LATE callback for A (the user finds the old
/// popup and finishes it anyway) must die at the claim: no
/// token exchange runs for it and no second account appears.
#[tokio::test]
async fn closed_popup_reopen_supersedes_abandoned_flow_then_completes() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();

    let abandoned_state = OpaqueStateHash::new(hex64(0x91)).unwrap();
    let abandoned_pkce = PkceVerifierHash::new(hex64(0x92)).unwrap();
    let abandoned_code = AuthorizationCodeHash::new(hex64(0x93)).unwrap();
    let abandoned_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &abandoned_state,
            &abandoned_pkce,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("the first Connect click must mint flow A");

    // The user closes the popup — nothing calls back — and clicks Connect
    // again. Minting flow B is itself the supersede seam.
    let reopened_state = OpaqueStateHash::new(hex64(0x94)).unwrap();
    let reopened_pkce = PkceVerifierHash::new(hex64(0x95)).unwrap();
    let reopened_code = AuthorizationCodeHash::new(hex64(0x96)).unwrap();
    let reopened_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &reopened_state,
            &reopened_pkce,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("re-opening the popup must mint flow B");
    let abandoned_record = bundle
        .services
        .flow_manager()
        .get_flow(&scope, abandoned_flow.id)
        .await
        .expect("get_flow must not error")
        .expect("the abandoned flow record must remain readable");
    assert_eq!(
        abandoned_record.state,
        AuthFlowState::Resolved(AuthFlowOutcome::UserAborted),
        "creating the reopened flow must supersede (cancel) the abandoned popup's flow"
    );

    // The reopened popup completes normally.
    let response = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            reopened_flow.id,
            &provider,
            &reopened_state,
            &reopened_pkce,
            &reopened_code,
            "Reopened Grant",
        ))
        .await
        .expect("the reopened popup's callback must complete");
    let reopened_account = response
        .credential_account_id
        .expect("the reopened callback must mint a credential account");
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "only the reopened flow's token exchange may cross the egress"
    );

    // The abandoned tab resurfaces and finishes the provider consent — the
    // late callback must be rejected at claim, before any exchange.
    let error = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            abandoned_flow.id,
            &provider,
            &abandoned_state,
            &abandoned_pkce,
            &abandoned_code,
            "Abandoned Grant",
        ))
        .await
        .expect_err("the superseded popup's late callback must be rejected");
    assert_eq!(
        error.code,
        AuthErrorCode::Canceled,
        "a superseded flow's callback must surface the canceled state, not a generic failure"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "no token exchange may run for the superseded popup's late callback"
    );
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(scope, provider))
        .await
        .expect("list_accounts must not error after the late callback");
    assert_eq!(
        page.accounts.len(),
        1,
        "the superseded popup must not mint a second credential account"
    );
    assert_eq!(
        page.accounts[0].id, reopened_account,
        "the surviving account is the reopened popup's account"
    );
}

/// Denied consent: the user clicks "Deny" on the provider page. The flow
/// terminalizes durably as Failed with no token exchange and no credential
/// account — the route-visible outcome is the sanitized non-retryable
/// `ProviderDenied` error — and an immediate fresh Connect succeeds cleanly
/// (denial leaves a clean retry path, not a wedge).
#[tokio::test]
async fn denied_consent_terminalizes_flow_and_fresh_retry_connects() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();

    let denied_state = OpaqueStateHash::new(hex64(0xa1)).unwrap();
    let denied_pkce = PkceVerifierHash::new(hex64(0xa2)).unwrap();
    let denied_flow = bundle
        .services
        .flow_manager()
        .create_flow(new_flow_request(
            &scope,
            &provider,
            &denied_state,
            &denied_pkce,
            Utc::now() + Duration::minutes(5),
        ))
        .await
        .expect("create_flow must succeed");

    let denial = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: denied_flow.id,
            opaque_state_hash: denied_state.clone(),
            outcome: RebornOAuthCallbackOutcome::ProviderDenied,
        })
        .await
        .expect_err("a denied consent surfaces as the sanitized ProviderDenied error");
    assert_eq!(
        denial.code,
        AuthErrorCode::ProviderDenied,
        "denied consent must render as ProviderDenied, not a generic failure"
    );
    assert!(
        !denial.retryable,
        "denied consent is terminal for THIS flow; retry means a fresh Connect"
    );
    let denied_record = bundle
        .services
        .flow_manager()
        .get_flow(&scope, denied_flow.id)
        .await
        .expect("get_flow must not error")
        .expect("the denied flow record must remain readable");
    assert_eq!(
        denied_record.state,
        AuthFlowState::Resolved(AuthFlowOutcome::ProviderDenied),
        "denied consent must preserve the provider-denied outcome durably"
    );
    assert_eq!(
        bundle.egress.captured_count(),
        0,
        "no token exchange may run for a denied consent"
    );

    // Immediate retry: a fresh Connect completes.
    let retry_state = OpaqueStateHash::new(hex64(0xa4)).unwrap();
    let retry_pkce = PkceVerifierHash::new(hex64(0xa5)).unwrap();
    let retry_code = AuthorizationCodeHash::new(hex64(0xa6)).unwrap();
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
        .expect("a fresh Connect after denial must mint a flow");
    let retry = bundle
        .services
        .handle_oauth_callback(authorized_callback_request(
            &scope,
            retry_flow.id,
            &provider,
            &retry_state,
            &retry_pkce,
            &retry_code,
            "Post-Denial Grant",
        ))
        .await
        .expect("the retry after denial must complete; denial must not wedge reconnects");
    let retry_account = retry
        .credential_account_id
        .expect("the retry must mint a credential account");
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(scope, provider))
        .await
        .expect("list_accounts must not error after the retry");
    assert_eq!(
        page.accounts.len(),
        1,
        "only the retry's account may exist after a denial"
    );
    assert_eq!(page.accounts[0].id, retry_account);
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "exactly the retry's token exchange may cross the egress"
    );
}
