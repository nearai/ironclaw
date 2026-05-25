//! OAuth protocol helpers shared by Reborn product auth providers.
//!
//! This module intentionally owns only protocol-level pieces: Google OAuth
//! constants, PKCE challenge construction, authorization URL assembly, and
//! redacted provider token projections. Durable flow state, callback routing,
//! provider exchange, and credential storage remain owned by the product auth
//! services in this crate.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    AuthProductError, AuthorizationCodeHash, OAuthAuthorizationCode, OAuthAuthorizationUrl,
    OpaqueStateHash, PkceVerifierHash, PkceVerifierSecret, ProviderScope,
};

/// Reborn auth provider id for Google OAuth accounts.
pub const GOOGLE_PROVIDER_ID: &str = "google";
/// Google OAuth 2.0 authorization endpoint.
pub const GOOGLE_AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google OAuth 2.0 token endpoint.
pub const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// Read-only access to Google Calendar calendars and events.
pub const GOOGLE_CALENDAR_READONLY_SCOPE: &str =
    "https://www.googleapis.com/auth/calendar.readonly";
/// Read/write access to Google Calendar events.
pub const GOOGLE_CALENDAR_EVENTS_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events";
/// Read-only access to Gmail messages and metadata.
pub const GOOGLE_GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
/// Permission to send Gmail messages.
pub const GOOGLE_GMAIL_SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";
/// Permission to modify Gmail messages and drafts.
pub const GOOGLE_GMAIL_MODIFY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";

/// URL-safe S256 PKCE code challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkceCodeChallenge(String);

impl PkceCodeChallenge {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PkceCodeChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Provider authorization URL input. This is protocol-only: callers still own
/// durable auth-flow records, callback routing, and provider exchange.
#[derive(Clone)]
pub struct OAuthAuthorizeUrlRequest<'a> {
    pub authorization_endpoint: &'a str,
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub code_challenge: &'a PkceCodeChallenge,
    pub scopes: &'a [ProviderScope],
    pub extra_params: &'a [(&'a str, &'a str)],
}

impl fmt::Debug for OAuthAuthorizeUrlRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthAuthorizeUrlRequest")
            .field("authorization_endpoint", &self.authorization_endpoint)
            .field("client_id", &"[REDACTED]")
            .field("redirect_uri", &self.redirect_uri)
            .field("state", &"[REDACTED]")
            .field("code_challenge", &"[REDACTED]")
            .field("scopes", &self.scopes)
            .field("extra_params", &self.extra_params)
            .finish()
    }
}

/// Redacted token-response projection after provider exchange. It can be used
/// by provider clients before converting token material into secret handles.
#[derive(Clone)]
pub struct OAuthTokenResponse {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub scopes: Vec<ProviderScope>,
    pub expires_in_seconds: Option<u64>,
}

impl fmt::Debug for OAuthTokenResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthTokenResponse")
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("scopes", &self.scopes)
            .field("expires_in_seconds", &self.expires_in_seconds)
            .finish()
    }
}

impl OAuthTokenResponse {
    pub fn new(
        access_token: SecretString,
        refresh_token: Option<SecretString>,
        scope_text: Option<&str>,
        expires_in_seconds: Option<u64>,
    ) -> Result<Self, AuthProductError> {
        if access_token.expose_secret().trim().is_empty() {
            return Err(AuthProductError::invalid_request(
                "oauth access token must not be empty",
            ));
        }
        if refresh_token
            .as_ref()
            .is_some_and(|token| token.expose_secret().trim().is_empty())
        {
            return Err(AuthProductError::invalid_request(
                "oauth refresh token must not be empty",
            ));
        }
        let scopes = scope_text
            .unwrap_or_default()
            .split_whitespace()
            .map(ProviderScope::new)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            access_token,
            refresh_token,
            scopes,
            expires_in_seconds,
        })
    }
}

pub fn opaque_state_hash(state: &str) -> Result<OpaqueStateHash, AuthProductError> {
    OpaqueStateHash::new(sha256_hex(state))
}

pub fn pkce_verifier_hash(
    verifier: &PkceVerifierSecret,
) -> Result<PkceVerifierHash, AuthProductError> {
    PkceVerifierHash::new(sha256_hex(verifier.expose_secret()))
}

pub fn authorization_code_hash(
    code: &OAuthAuthorizationCode,
) -> Result<AuthorizationCodeHash, AuthProductError> {
    AuthorizationCodeHash::new(sha256_hex(code.expose_secret()))
}

pub fn pkce_s256_challenge(verifier: &PkceVerifierSecret) -> PkceCodeChallenge {
    PkceCodeChallenge(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose_secret().as_bytes())))
}

pub fn build_authorization_url(
    request: OAuthAuthorizeUrlRequest<'_>,
) -> Result<OAuthAuthorizationUrl, AuthProductError> {
    validate_authorize_fragment("oauth client id", request.client_id)?;
    validate_authorize_fragment("oauth redirect uri", request.redirect_uri)?;
    validate_authorize_fragment("oauth state", request.state)?;

    let mut url = Url::parse(request.authorization_endpoint).map_err(|_| {
        AuthProductError::invalid_request("oauth authorization endpoint must be an absolute url")
    })?;
    if url.scheme() != "https" {
        return Err(AuthProductError::invalid_request(
            "oauth authorization endpoint must use https",
        ));
    }
    if url.host_str().is_none() {
        return Err(AuthProductError::invalid_request(
            "oauth authorization endpoint host is required",
        ));
    }
    for (name, _) in url.query_pairs() {
        if is_reserved_authorize_param(name.as_ref()) {
            return Err(AuthProductError::invalid_request(
                "oauth authorization endpoint must not predefine reserved query parameters",
            ));
        }
    }

    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("client_id", request.client_id)
            .append_pair("redirect_uri", request.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &scope_text(request.scopes))
            .append_pair("state", request.state)
            .append_pair("code_challenge", request.code_challenge.as_str())
            .append_pair("code_challenge_method", "S256");
        for (name, value) in request.extra_params {
            validate_extra_param_name(name)?;
            validate_authorize_fragment("oauth query parameter value", value)?;
            pairs.append_pair(name, value);
        }
    }

    OAuthAuthorizationUrl::new(url.to_string())
}

pub fn build_google_authorization_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &PkceCodeChallenge,
    scopes: &[ProviderScope],
    hosted_domain_hint: Option<&str>,
) -> Result<OAuthAuthorizationUrl, AuthProductError> {
    let mut extra_params = vec![
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("include_granted_scopes", "true"),
    ];
    if let Some(hosted_domain) = hosted_domain_hint {
        validate_authorize_fragment("google hosted domain", hosted_domain)?;
        extra_params.push(("hd", hosted_domain));
    }
    build_authorization_url(OAuthAuthorizeUrlRequest {
        authorization_endpoint: GOOGLE_AUTHORIZATION_ENDPOINT,
        client_id,
        redirect_uri,
        state,
        code_challenge,
        scopes,
        extra_params: &extra_params,
    })
}

pub fn scope_text(scopes: &[ProviderScope]) -> String {
    scopes
        .iter()
        .map(ProviderScope::as_str)
        .collect::<Vec<_>>()
        .join(" ")
}

fn sha256_hex(value: &str) -> String {
    // Digest newtypes expose constant-time comparison for callback/state checks;
    // this helper only constructs the canonical 64-character lowercase hex text.
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn validate_authorize_fragment(label: &'static str, value: &str) -> Result<(), AuthProductError> {
    if value.trim().is_empty() {
        return Err(AuthProductError::invalid_request(format!(
            "{label} must not be empty"
        )));
    }
    if value
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(AuthProductError::invalid_request(format!(
            "{label} must not contain NUL/control characters"
        )));
    }
    Ok(())
}

fn validate_extra_param_name(name: &str) -> Result<(), AuthProductError> {
    validate_authorize_fragment("oauth query parameter name", name)?;
    if is_reserved_authorize_param(name) {
        return Err(AuthProductError::invalid_request(
            "oauth query parameter name is reserved",
        ));
    }
    Ok(())
}

fn is_reserved_authorize_param(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "client_id"
            | "redirect_uri"
            | "response_type"
            | "scope"
            | "state"
            | "code_challenge"
            | "code_challenge_method"
    )
}
