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

use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthErrorCode, AuthFlowId, AuthFlowKind, AuthProductScope,
    AuthProviderId, AuthSurface, AuthorizationCodeHash, CredentialAccountLabel,
    CredentialAccountLookupRequest, NewAuthFlow, OAuthAuthorizationCode, OAuthAuthorizationUrl,
    OAuthProviderCallbackRequest, OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret,
    ProviderScope,
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
    let expires_at = Utc::now() + Duration::minutes(5);

    // Step 1 — create an OAuth flow in the real durable store.
    let flow = bundle
        .services
        .flow_manager()
        .create_flow(NewAuthFlow {
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
        })
        .await
        .expect("create_flow must succeed");

    // Step 2 — drive the OAuth callback: claim → token exchange → complete.
    // The scripted egress returns a fixed access-token JSON body; no real
    // network call is made.
    let response = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider.clone(),
                    account_label: CredentialAccountLabel::new("Test Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "raw-auth-code-value".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "raw-pkce-verifier-value".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash,
                    scopes: vec![ProviderScope::new("test.readonly").unwrap()],
                },
            },
        })
        .await
        .expect("handle_oauth_callback must succeed");

    // Step 3 — assert the callback produced a credential account id.
    let account_id = response
        .credential_account_id
        .expect("completed callback must carry a credential_account_id");

    // Step 4 — read back the persisted account from the real store.
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

    // Step 5 — verify exactly one token-exchange HTTP call was captured.
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "exactly one token-exchange HTTP call must be captured by the scripted egress"
    );

    // Step 6 — verify the connect exchange used the authorization_code grant
    // (not the refresh grant) — proves the right OAuth flow crossed the egress.
    let bodies = bundle.egress.captured_bodies();
    let body = String::from_utf8_lossy(&bodies[0]);
    assert!(
        body.contains("authorization_code"),
        "connect-flow token exchange must use the authorization_code grant; body: {body}"
    );
}

/// Guard test: attempting an OAuth callback for a non-existent flow must fail
/// with `UnknownOrExpiredFlow`.  No credential account must be created, and no
/// token-exchange call should be made.
#[tokio::test]
async fn oauth_callback_without_prior_flow_fails() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let state_hash = OpaqueStateHash::new(hex64(0xdd)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0xee)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0xff)).unwrap();

    let error = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope,
            flow_id: AuthFlowId::new(), // no flow was created for this id
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: AuthProviderId::new("test-oauth-provider").unwrap(),
                    account_label: CredentialAccountLabel::new("Guard Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "guard-auth-code".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "guard-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash,
                    scopes: vec![],
                },
            },
        })
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
}
