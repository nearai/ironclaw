//! Shared fixtures for the auth-journey integration bins in this folder:
//! scope/flow/callback constructors over the real
//! `FilesystemAuthProductServices<InMemoryBackend>` product-auth bundle with
//! the token-exchange HTTP captured by `ScriptedOAuthTokenEgress`.
//!
//! Mounted per-bin via `#[path = "common.rs"] mod common;` — each test binary
//! compiles its own copy, so keep this file dependency-light.

use chrono::{DateTime, Utc};
use ironclaw_auth::{
    AuthChallenge, AuthContinuationRef, AuthFlowId, AuthFlowKind, AuthProductScope, AuthProviderId,
    AuthSurface, AuthorizationCodeHash, CredentialAccountLabel, NewAuthFlow,
    OAuthAuthorizationCode, OAuthAuthorizationUrl, OAuthProviderCallbackRequest, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope,
};
use ironclaw_auth::{RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest};
use ironclaw_host_api::{InvocationId, ResourceScope, UserId};
use secrecy::SecretString;

/// Build a 64-character hex string from a repeated byte value.
pub fn hex64(fill: u8) -> String {
    format!("{fill:02x}").repeat(32)
}

pub fn test_scope() -> AuthProductScope {
    let resource =
        ResourceScope::local_default(UserId::new("test-user").unwrap(), InvocationId::new())
            .expect("local_default scope must build");
    AuthProductScope::new(resource, AuthSurface::Callback)
}

/// A `NewAuthFlow` for the connect-flow tests; only identity hashes and the
/// expiry vary between scenarios.
pub fn new_flow_request(
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
pub fn authorized_callback_request(
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
