use super::common::*;
use ironclaw_auth::{
    GOOGLE_AUTHORIZATION_ENDPOINT, GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
    OAuthAuthorizationCode, OAuthAuthorizeUrlRequest, OAuthTokenResponse, PkceCodeChallenge,
    PkceVerifierSecret, build_authorization_url, build_google_authorization_url, opaque_state_hash,
    pkce_s256_challenge, pkce_verifier_hash, scope_text,
};
use secrecy::ExposeSecret;

fn verifier_challenge() -> PkceCodeChallenge {
    let verifier = PkceVerifierSecret::new(secret("raw-pkce-verifier")).unwrap();
    pkce_s256_challenge(&verifier)
}

fn valid_scopes() -> Vec<ironclaw_auth::ProviderScope> {
    provider_scopes(&[GOOGLE_CALENDAR_READONLY_SCOPE])
}

fn valid_authorization_request<'a>(
    challenge: &'a PkceCodeChallenge,
    scopes: &'a [ironclaw_auth::ProviderScope],
    extra_params: &'a [(&'a str, &'a str)],
) -> OAuthAuthorizeUrlRequest<'a> {
    OAuthAuthorizeUrlRequest {
        authorization_endpoint: GOOGLE_AUTHORIZATION_ENDPOINT,
        client_id: "client-id.apps.googleusercontent.com",
        redirect_uri: "http://127.0.0.1:5555/oauth/callback/google",
        state: "opaque-state",
        code_challenge: challenge,
        scopes,
        extra_params,
    }
}

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
fn authorization_url_builder_sets_core_oauth_parameters() {
    let verifier = PkceVerifierSecret::new(secret("raw-pkce-verifier")).unwrap();
    let challenge = pkce_s256_challenge(&verifier);
    let scopes = provider_scopes(&[GOOGLE_CALENDAR_READONLY_SCOPE, GOOGLE_GMAIL_SEND_SCOPE]);

    let url = build_authorization_url(OAuthAuthorizeUrlRequest {
        authorization_endpoint: GOOGLE_AUTHORIZATION_ENDPOINT,
        client_id: "client-id.apps.googleusercontent.com",
        redirect_uri: "http://127.0.0.1:5555/oauth/callback/google",
        state: "opaque-state",
        code_challenge: &challenge,
        scopes: &scopes,
        extra_params: &[("access_type", "offline")],
    })
    .unwrap();
    let parsed = url::Url::parse(url.as_str()).unwrap();

    assert_eq!(parsed.scheme(), "https");
    assert_eq!(parsed.host_str(), Some("accounts.google.com"));
    let query = parsed.query_pairs().collect::<Vec<_>>();
    assert!(query.iter().any(|(name, value)| {
        name == "scope"
            && value == "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/gmail.send"
    }));
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "code_challenge" && value == challenge.as_str())
    );
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "code_challenge_method" && value == "S256")
    );
}

#[test]
fn google_authorization_url_includes_google_offline_consent_defaults() {
    let verifier = PkceVerifierSecret::new(secret("raw-pkce-verifier")).unwrap();
    let challenge = pkce_s256_challenge(&verifier);
    let scopes = provider_scopes(&[GOOGLE_GMAIL_SEND_SCOPE]);

    let url = build_google_authorization_url(
        "client-id.apps.googleusercontent.com",
        "http://127.0.0.1:5555/oauth/callback/google",
        "opaque-state",
        &challenge,
        &scopes,
        Some("near.ai"),
    )
    .unwrap();
    let parsed = url::Url::parse(url.as_str()).unwrap();
    let query = parsed.query_pairs().collect::<Vec<_>>();

    assert!(
        query
            .iter()
            .any(|(name, value)| name == "access_type" && value == "offline")
    );
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "prompt" && value == "consent")
    );
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "include_granted_scopes" && value == "true")
    );
    assert!(
        query
            .iter()
            .any(|(name, value)| name == "hd" && value == "near.ai")
    );
}

#[test]
fn token_response_projects_scopes_and_redacts_debug() {
    let response = OAuthTokenResponse::new(
        secret("access-token"),
        Some(secret("refresh-token")),
        Some("https://www.googleapis.com/auth/gmail.send https://www.googleapis.com/auth/calendar.readonly"),
        Some(3600),
    )
    .unwrap();

    assert_eq!(response.access_token.expose_secret(), "access-token");
    assert_eq!(
        scope_text(&response.scopes),
        "https://www.googleapis.com/auth/gmail.send https://www.googleapis.com/auth/calendar.readonly"
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
fn authorization_url_request_debug_redacts_sensitive_fields() {
    let challenge = verifier_challenge();
    let scopes = valid_scopes();
    let request = valid_authorization_request(&challenge, &scopes, &[]);

    let debug = format!("{request:?}");

    assert!(!debug.contains("client-id.apps.googleusercontent.com"));
    assert!(!debug.contains("opaque-state"));
    assert!(!debug.contains(challenge.as_str()));
    assert!(debug.contains("[REDACTED]"));
}

#[test]
fn authorization_url_builder_rejects_missing_core_fields() {
    let challenge = verifier_challenge();
    let scopes = valid_scopes();

    assert_invalid_request(build_authorization_url(OAuthAuthorizeUrlRequest {
        client_id: "",
        ..valid_authorization_request(&challenge, &scopes, &[])
    }));
    assert_invalid_request(build_authorization_url(OAuthAuthorizeUrlRequest {
        redirect_uri: "",
        ..valid_authorization_request(&challenge, &scopes, &[])
    }));
    assert_invalid_request(build_authorization_url(OAuthAuthorizeUrlRequest {
        state: "",
        ..valid_authorization_request(&challenge, &scopes, &[])
    }));
}

#[test]
fn authorization_url_builder_rejects_bad_endpoint_urls() {
    let challenge = verifier_challenge();
    let scopes = valid_scopes();

    assert_invalid_request(build_authorization_url(OAuthAuthorizeUrlRequest {
        authorization_endpoint: "not a url",
        ..valid_authorization_request(&challenge, &scopes, &[])
    }));
    assert_invalid_request(build_authorization_url(OAuthAuthorizeUrlRequest {
        authorization_endpoint: "http://accounts.google.com/o/oauth2/v2/auth",
        ..valid_authorization_request(&challenge, &scopes, &[])
    }));
    assert_invalid_request(build_authorization_url(OAuthAuthorizeUrlRequest {
        authorization_endpoint: "https://user:pass@accounts.google.com/o/oauth2/v2/auth",
        ..valid_authorization_request(&challenge, &scopes, &[])
    }));
    assert_invalid_request(build_authorization_url(OAuthAuthorizeUrlRequest {
        authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth?state=predefined",
        ..valid_authorization_request(&challenge, &scopes, &[])
    }));
}

#[test]
fn authorization_url_builder_rejects_bad_extra_params() {
    let challenge = verifier_challenge();
    let scopes = valid_scopes();

    assert_invalid_request(build_authorization_url(valid_authorization_request(
        &challenge,
        &scopes,
        &[("", "value")],
    )));
    assert_invalid_request(build_authorization_url(valid_authorization_request(
        &challenge,
        &scopes,
        &[("login_hint", "")],
    )));
    assert_invalid_request(build_authorization_url(valid_authorization_request(
        &challenge,
        &scopes,
        &[("login_hint", "bad\u{0000}value")],
    )));
    assert_invalid_request(build_authorization_url(valid_authorization_request(
        &challenge,
        &scopes,
        &[("state", "override")],
    )));
    assert_invalid_request(build_authorization_url(valid_authorization_request(
        &challenge,
        &scopes,
        &[("redirect_uri", "https://attacker.example/callback")],
    )));
}

#[test]
fn authorization_url_builder_handles_empty_scopes_and_extra_params() {
    let challenge = verifier_challenge();
    let scopes = Vec::new();

    let url =
        build_authorization_url(valid_authorization_request(&challenge, &scopes, &[])).unwrap();
    let parsed = url::Url::parse(url.as_str()).unwrap();
    let query = parsed.query_pairs().collect::<Vec<_>>();

    assert!(
        query
            .iter()
            .any(|(name, value)| name == "scope" && value.is_empty())
    );
    assert!(!query.iter().any(|(name, _)| name == "access_type"));
}

#[test]
fn google_authorization_url_rejects_invalid_hosted_domain() {
    let challenge = verifier_challenge();
    let scopes = valid_scopes();

    assert_invalid_request(build_google_authorization_url(
        "client-id.apps.googleusercontent.com",
        "http://127.0.0.1:5555/oauth/callback/google",
        "opaque-state",
        &challenge,
        &scopes,
        Some("near.ai\nexample.com"),
    ));
}
