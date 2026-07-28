//! Contract pins for the OAuth protocol helpers that survive the recipe
//! engine: hash helpers, PKCE challenge construction, validated OAuth value
//! newtypes, callback-state encode/decode, and the redacted token-response
//! projection. Authorization-URL construction is engine behavior, pinned by
//! `crates/ironclaw_auth/tests/auth_engine_contract.rs`.

use super::common::*;
use ironclaw_auth::{
    AuthFlowId, OAuthAuthorizationCode, OAuthCallbackState, OAuthCallbackStateKind, OAuthClientId,
    OAuthRedirectUri, OAuthState, OAuthTokenResponse, PkceVerifierSecret, ProviderScope,
    opaque_state_hash, pkce_s256_challenge, pkce_verifier_hash, scope_text,
};
use secrecy::ExposeSecret;

fn assert_invalid_request<T>(result: Result<T, ironclaw_auth::AuthProductError>) {
    let error = result.err().expect("request should be invalid");
    assert_eq!(error.code(), AuthErrorCode::InvalidRequest);
}

#[test]
fn oauth_hash_helpers_emit_valid_stable_digests_without_raw_material() {
    let verifier = PkceVerifierSecret::new(secret("raw-pkce-verifier")).unwrap();
    let code = OAuthAuthorizationCode::new(secret("raw-auth-code")).unwrap();

    let state_hash = opaque_state_hash("opaque-state").unwrap();
    let verifier_hash = pkce_verifier_hash(&verifier).unwrap();
    let code_hash = ironclaw_auth::authorization_code_hash(&code).unwrap();

    assert_eq!(state_hash.as_str().len(), 64);
    assert_eq!(verifier_hash.as_str().len(), 64);
    assert_eq!(code_hash.as_str().len(), 64);
    assert_ne!(state_hash.as_str(), "opaque-state");
    assert_ne!(verifier_hash.as_str(), "raw-pkce-verifier");
    assert_ne!(code_hash.as_str(), "raw-auth-code");
}

#[test]
fn pkce_s256_challenge_uses_url_safe_base64_without_padding() {
    let verifier = PkceVerifierSecret::new(secret("correct horse battery staple")).unwrap();

    let challenge = pkce_s256_challenge(&verifier);

    assert!(!challenge.as_str().contains('='));
    assert!(!challenge.as_str().contains('+'));
    assert!(!challenge.as_str().contains('/'));
    assert_eq!(challenge.as_str().len(), 43);
}

#[test]
fn recipe_callback_state_round_trips_and_rejects_foreign_prefixes() {
    let scope = auth_scope();
    let label = account_label("acct");
    let flow_id = AuthFlowId::new();
    let scopes = provider_scopes(&["items:read"]);

    let encoded = OAuthCallbackState::new(
        OAuthCallbackStateKind::RECIPE,
        flow_id,
        scope,
        label.clone(),
        scopes.clone(),
    )
    .unwrap()
    .encode()
    .unwrap();

    assert!(encoded.as_str().starts_with("icr1."));
    let decoded =
        OAuthCallbackState::decode(OAuthCallbackStateKind::RECIPE, encoded.as_str()).unwrap();
    assert_eq!(decoded.flow_id(), flow_id);
    assert_eq!(decoded.account_label(), &label);
    assert_eq!(decoded.requested_scopes(), scopes.as_slice());

    // Values without the recipe prefix must not decode.
    let error = OAuthCallbackState::decode(OAuthCallbackStateKind::RECIPE, "icg1.someoldstate")
        .unwrap_err();
    assert_eq!(error.code(), AuthErrorCode::MalformedCallback);
}

#[test]
fn token_response_projects_scopes_and_redacts_debug() {
    let response = OAuthTokenResponse::new(
        secret("access-token"),
        Some(secret("refresh-token")),
        Some("vendor.scope.one vendor.scope.two"),
        Some(3600),
    )
    .unwrap();

    assert_eq!(response.access_token.expose_secret(), "access-token");
    assert_eq!(
        scope_text(&response.scopes),
        "vendor.scope.one vendor.scope.two"
    );
    let debug = format!("{response:?}");
    assert!(!debug.contains("access-token"));
    assert!(!debug.contains("refresh-token"));
}

#[test]
fn token_response_rejects_empty_token_material() {
    let error = OAuthTokenResponse::new(secret(""), None, None, None).unwrap_err();

    assert_eq!(error.code(), AuthErrorCode::InvalidRequest);
}

#[test]
fn token_response_rejects_empty_refresh_token() {
    assert_invalid_request(OAuthTokenResponse::new(
        secret("access-token"),
        Some(secret("")),
        None,
        None,
    ));
}

#[test]
fn token_response_rejects_invalid_scope() {
    assert_invalid_request(OAuthTokenResponse::new(
        secret("access-token"),
        None,
        Some("valid-scope \0invalid"),
        None,
    ));
}

#[test]
fn oauth_value_newtypes_reject_empty_and_control_values() {
    assert_invalid_request(OAuthClientId::new(""));
    assert_invalid_request(OAuthRedirectUri::new(""));
    assert_invalid_request(OAuthState::new(""));
    assert_invalid_request(OAuthState::new("opaque\nstate"));
}

#[test]
fn oauth_value_newtypes_redact_debug() {
    let client_id = OAuthClientId::new("client-id.apps.example").unwrap();
    let state = OAuthState::new("opaque-state").unwrap();
    assert_eq!(format!("{client_id:?}"), "[REDACTED]");
    assert_eq!(format!("{state:?}"), "[REDACTED]");
}

#[test]
fn scope_text_returns_empty_string_for_empty_scopes() {
    assert!(scope_text(&[]).is_empty());
}

fn auth_scope() -> ironclaw_auth::AuthProductScope {
    ironclaw_auth::AuthProductScope::new(
        ironclaw_host_api::ResourceScope {
            tenant_id: ironclaw_host_api::TenantId::new("tenant-a").unwrap(),
            user_id: ironclaw_host_api::UserId::new("user-a").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        },
        ironclaw_auth::AuthSurface::Callback,
    )
}

fn account_label(value: &str) -> ironclaw_auth::CredentialAccountLabel {
    ironclaw_auth::CredentialAccountLabel::new(value).unwrap()
}

fn provider_scopes(values: &[&str]) -> Vec<ProviderScope> {
    values
        .iter()
        .map(|value| ProviderScope::new(value.to_string()).unwrap())
        .collect()
}
