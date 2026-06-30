//! Shared Google OAuth connect-flow helper for Reborn integration tests.
//!
//! Provides [`connect_google_account`], the standard OAuth connect flow that
//! drives `create_flow` → `handle_oauth_callback` → `get_account` to produce
//! a connected `CredentialAccount`.  Factored out because multiple test files
//! exercise paths that require a pre-connected Google credential account.
//!
//! The function and its dependencies are gated on
//! `any(feature = "libsql", feature = "postgres")` to match the gate on
//! `OAuthProductAuthTestBundle` in `ironclaw_reborn_composition::test_support`.

// Shared support module: not every test binary that mounts the `reborn_support`
// tree calls into this helper (e.g. `support_unit_tests` exercises none of it),
// so its symbols read as dead there under `-D warnings`. Module-level allow
// matches `builder.rs`/`assertions.rs`.
#![allow(dead_code)]

#[cfg(any(feature = "libsql", feature = "postgres"))]
use chrono::{Duration, Utc};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthProductScope, AuthProviderId,
    AuthorizationCodeHash, CredentialAccountLabel, CredentialAccountLookupRequest, NewAuthFlow,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_reborn_composition::{
    RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    test_support::OAuthProductAuthTestBundle,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use secrecy::SecretString;

/// Build a 64-character hex string from a repeated byte value.
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn hex64(fill: u8) -> String {
    format!("{fill:02x}").repeat(32)
}

/// Run the standard Google OAuth connect flow on `bundle` and return the
/// persisted `CredentialAccount`.
///
/// Drives `create_flow` → `handle_oauth_callback` → `get_account` using
/// `fill` as a seed byte to generate deterministic hex hashes for the OAuth
/// state, PKCE verifier, and authorization code.  Call with distinct `fill`
/// values when multiple accounts are needed in the same test.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub async fn connect_google_account(
    bundle: &OAuthProductAuthTestBundle,
    scope: &AuthProductScope,
    fill: u8,
) -> ironclaw_auth::CredentialAccount {
    let provider = AuthProviderId::new("google").unwrap();
    let state_hash = OpaqueStateHash::new(hex64(fill)).unwrap();
    let pkce_hash = PkceVerifierHash::new(hex64(fill.wrapping_add(1))).unwrap();
    let code_hash = AuthorizationCodeHash::new(hex64(fill.wrapping_add(2))).unwrap();
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
                    "https://accounts.google.com/o/oauth2/auth",
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

    let response = bundle
        .services
        .handle_oauth_callback(RebornOAuthCallbackRequest {
            scope: scope.clone(),
            flow_id: flow.id,
            opaque_state_hash: state_hash,
            outcome: RebornOAuthCallbackOutcome::Authorized {
                provider_request: OAuthProviderCallbackRequest {
                    provider: provider.clone(),
                    account_label: CredentialAccountLabel::new("Google Account").unwrap(),
                    authorization_code: OAuthAuthorizationCode::new(SecretString::from(
                        "google-auth-code".to_string(),
                    ))
                    .unwrap(),
                    authorization_code_hash: code_hash,
                    pkce_verifier: PkceVerifierSecret::new(SecretString::from(
                        "google-pkce-verifier".to_string(),
                    ))
                    .unwrap(),
                    pkce_verifier_hash: pkce_hash,
                    scopes: vec![ProviderScope::new("email").unwrap()],
                },
            },
        })
        .await
        .expect("handle_oauth_callback must succeed");

    let account_id = response
        .credential_account_id
        .expect("completed callback must carry a credential_account_id");

    bundle
        .services
        .credential_account_service()
        .get_account(CredentialAccountLookupRequest::new(
            scope.clone(),
            account_id,
        ))
        .await
        .expect("get_account must not error")
        .expect("credential account must be persisted after a successful OAuth callback")
}
