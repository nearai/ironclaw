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

use chrono::{Duration, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowKind, AuthProductScope, AuthProviderId,
    AuthSurface, AuthorizationCodeHash, CredentialAccountLookupRequest, NewAuthFlow,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope,
};
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use ironclaw_reborn_composition::{
    CredentialRefreshSettings, RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest,
    test_support::build_google_oauth_product_auth_for_test,
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

/// Run the standard Google OAuth connect flow and return the persisted
/// `CredentialAccount` from the store.
async fn connect_google_account(
    bundle: &ironclaw_reborn_composition::test_support::OAuthProductAuthTestBundle,
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
                    account_label: ironclaw_auth::CredentialAccountLabel::new("Google Account")
                        .unwrap(),
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

/// Positive test: a sweep with a frozen clock 3 days ahead (past the 2-day
/// idle threshold) triggers a token-refresh HTTP call for the idle account.
#[tokio::test]
async fn credential_refresh_sweep_refreshes_idle_google_account() {
    let bundle = build_google_oauth_product_auth_for_test();
    let scope = test_scope();

    // Step 1 — run the OAuth connect flow to create a Google credential account.
    // After this, egress.captured_count() == 1 (initial token exchange).
    let account = connect_google_account(&bundle, &scope, 0xaa).await;

    // Step 2 — freeze the clock 3 days ahead.  The account was just created
    // (updated_at ≈ Utc::now()), so idle_cutoff = frozen_now − 2 days is
    // still 1 day in the future relative to creation, making the account idle.
    let frozen_now = Utc::now() + Duration::days(3);

    // Step 3 — run the sweep with the frozen clock and an enabled settings bundle.
    bundle
        .sweep_for_refresh(
            vec![account],
            CredentialRefreshSettings::enabled(),
            frozen_now,
        )
        .await;

    // Step 4 — egress must now have captured 2 calls: the initial token exchange
    // and the refresh call from the sweep.
    assert_eq!(
        bundle.egress.captured_count(),
        2,
        "sweep must trigger exactly one refresh HTTP call for the idle account \
         (total egress count: initial exchange + refresh)"
    );
}

/// Guard test: a sweep with the real clock does NOT refresh a freshly-created
/// account that is still within the 2-day idle threshold.
#[tokio::test]
async fn credential_refresh_sweep_skips_fresh_google_account() {
    let bundle = build_google_oauth_product_auth_for_test();
    let scope = test_scope();

    // Step 1 — create the account (egress count becomes 1).
    let account = connect_google_account(&bundle, &scope, 0xbb).await;

    // Step 2 — sweep with Utc::now() as the clock.  The account was just
    // created, so updated_at is effectively Utc::now(); idle_cutoff = now −
    // 2 days is 2 days ago, which is BEFORE updated_at → account is NOT idle.
    bundle
        .sweep_for_refresh(
            vec![account],
            CredentialRefreshSettings::enabled(),
            Utc::now(),
        )
        .await;

    // Step 3 — no refresh call should have been made.
    assert_eq!(
        bundle.egress.captured_count(),
        1,
        "sweep must not refresh a freshly-created account that is still within \
         the idle threshold (egress count must stay at 1, the initial exchange)"
    );
}
