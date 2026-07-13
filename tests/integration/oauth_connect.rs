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

    // Drives claim → token exchange → complete. The scripted egress returns a
    // fixed access-token JSON body; no real network call is made.
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

    // Recipe-engine pins (names only — values carry secrets): the exchange
    // body is host-constructed per the vendor recipe, carrying the PKCE
    // verifier, the deployment client id, and the static vendor redirect.
    let param_names = bundle.egress.captured_form_param_names();
    let exchange_params = param_names.first().expect("one captured exchange");
    for expected in [
        "grant_type",
        "code",
        "code_verifier",
        "client_id",
        "redirect_uri",
    ] {
        assert!(
            exchange_params.iter().any(|name| name == expected),
            "engine exchange body must carry `{expected}`; got {exchange_params:?}"
        );
    }
}

/// AUTH-4 at the integration tier: a callback requesting scopes beyond the
/// vendor recipe's ceiling is rejected by the engine BEFORE any vendor call —
/// no token-exchange egress, no persisted credential account.
#[tokio::test]
async fn oauth_connect_rejects_scopes_beyond_the_recipe_ceiling() {
    let bundle = build_oauth_product_auth_for_test();
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();
    let state_hash = OpaqueStateHash::new(hex64(0x21)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x22)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x23)).unwrap();
    let expires_at = Utc::now() + Duration::minutes(5);

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

    let error = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider.clone(),
                    account_label: CredentialAccountLabel::new("Ceiling Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "widened-auth-code".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "widened-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier_hash: pkce_hash,
                    // The bundle recipe's ceiling is ["test.readonly"].
                    scopes: vec![ProviderScope::new("test.admin").unwrap()],
                },
            },
        })
        .await
        .expect_err("scopes beyond the recipe ceiling must be rejected");

    assert_eq!(error.code, AuthErrorCode::InvalidRequest);
    assert_eq!(
        bundle.egress.captured_count(),
        0,
        "widening must be rejected before the vendor call"
    );
    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(scope, provider))
        .await
        .expect("list_accounts must not error");
    assert!(
        page.accounts.is_empty(),
        "no credential account must be created for a rejected exchange"
    );
}

/// The same connect flow persisted through the libSQL-backed durable
/// flow/account store — the auth engine's second persistence backend
/// (checklist AUTH-15; the primary suite runs on the in-memory backend).
#[cfg(feature = "libsql")]
#[tokio::test]
async fn oauth_connect_flow_persists_credential_account_on_libsql() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bundle =
        ironclaw_reborn_composition::test_support::build_oauth_product_auth_for_test_on_libsql(
            &dir.path().join("oauth-connect.db"),
        )
        .await;
    let scope = test_scope();
    let provider = AuthProviderId::new("test-oauth-provider").unwrap();
    let state_hash = OpaqueStateHash::new(hex64(0x31)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(0x32)).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(0x33)).unwrap();
    let expires_at = Utc::now() + Duration::minutes(5);

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
        .expect("create_flow must succeed on libsql");

    let response = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider.clone(),
                    account_label: CredentialAccountLabel::new("LibSql Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "libsql-auth-code".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "libsql-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash,
                    scopes: vec![ProviderScope::new("test.readonly").unwrap()],
                },
            },
        })
        .await
        .expect("handle_oauth_callback must succeed on libsql");

    let account_id = response
        .credential_account_id
        .expect("completed callback must carry a credential_account_id");
    let account = bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(scope, account_id))
        .await
        .expect("get_account must not error")
        .expect("credential account must persist on the libsql backend");
    assert_eq!(account.provider, provider);
    assert_eq!(
        bundle
            .egress
            .captured_grant_types()
            .first()
            .map(String::as_str),
        Some("authorization_code")
    );
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
    // Clone scope before it is moved into the callback request so we can use
    // it for the list_accounts assertion after the error is returned.
    let scope_for_assert = scope.clone();
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

    let page = bundle
        .services
        .credential_account_service()
        .list_accounts(CredentialAccountListRequest::new(
            scope_for_assert,
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
