//! OAuth protocol helpers shared by Reborn product auth providers.
//!
//! This module intentionally owns only protocol-level pieces: Google OAuth
//! constants, PKCE challenge construction, authorization URL assembly, and
//! redacted provider token projections. Durable flow state, callback routing,
//! provider exchange, and credential storage remain owned by the product auth
//! services in this crate.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};
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

/// Validated OAuth authorization endpoint.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthAuthorizationEndpoint(String);

impl OAuthAuthorizationEndpoint {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        let value = value.into();
        let url = Url::parse(&value).map_err(|_| {
            AuthProductError::invalid_request(
                "oauth authorization endpoint must be an absolute url",
            )
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
        if !url.username().is_empty() || url.password().is_some() {
            return Err(AuthProductError::invalid_request(
                "oauth authorization endpoint must not include userinfo",
            ));
        }
        for (name, _) in url.query_pairs() {
            if is_reserved_authorize_param(name.as_ref()) {
                return Err(AuthProductError::invalid_request(
                    "oauth authorization endpoint must not predefine reserved query parameters",
                ));
            }
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OAuthAuthorizationEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Validated OAuth client id.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthClientId(String);

impl OAuthClientId {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        let value = value.into();
        validate_authorize_fragment("oauth client id", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OAuthClientId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Validated OAuth redirect URI.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthRedirectUri(String);

impl OAuthRedirectUri {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        let value = value.into();
        validate_authorize_fragment("oauth redirect uri", &value)?;
        let url = Url::parse(&value)
            .map_err(|_| AuthProductError::invalid_request("oauth redirect uri must be a url"))?;
        let is_loopback_http = url.scheme() == "http"
            && url
                .host_str()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "[::1]"));
        if url.scheme() != "https" && !is_loopback_http {
            return Err(AuthProductError::invalid_request(
                "oauth redirect uri must use https unless it targets loopback localhost",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OAuthRedirectUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Validated opaque OAuth state value.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthState(String);

impl OAuthState {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        let value = value.into();
        validate_authorize_fragment("oauth state", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OAuthState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Validated provider-specific OAuth authorization query parameter.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthExtraParam {
    name: String,
    value: String,
}

impl OAuthExtraParam {
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, AuthProductError> {
        let name = name.into();
        let value = value.into();
        validate_extra_param_name(&name)?;
        validate_authorize_fragment("oauth query parameter value", &value)?;
        Ok(Self { name, value })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

impl fmt::Debug for OAuthExtraParam {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthExtraParam")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// Provider authorization URL input. This is protocol-only: callers still own
/// durable auth-flow records, callback routing, and provider exchange.
#[derive(Clone)]
pub struct OAuthAuthorizeUrlRequest<'a> {
    pub authorization_endpoint: &'a OAuthAuthorizationEndpoint,
    pub client_id: &'a OAuthClientId,
    pub redirect_uri: &'a OAuthRedirectUri,
    pub state: &'a OAuthState,
    pub code_challenge: &'a PkceCodeChallenge,
    pub scopes: &'a [ProviderScope],
    pub extra_params: &'a [OAuthExtraParam],
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
    OpaqueStateHash::new(ironclaw_common::hashing::sha256_hex(state.as_bytes()))
}

pub fn pkce_verifier_hash(
    verifier: &PkceVerifierSecret,
) -> Result<PkceVerifierHash, AuthProductError> {
    PkceVerifierHash::new(ironclaw_common::hashing::sha256_hex(
        verifier.expose_secret().as_bytes(),
    ))
}

pub fn authorization_code_hash(
    code: &OAuthAuthorizationCode,
) -> Result<AuthorizationCodeHash, AuthProductError> {
    AuthorizationCodeHash::new(ironclaw_common::hashing::sha256_hex(
        code.expose_secret().as_bytes(),
    ))
}

pub fn pkce_s256_challenge(verifier: &PkceVerifierSecret) -> PkceCodeChallenge {
    PkceCodeChallenge(ironclaw_common::pkce::s256_challenge(
        verifier.expose_secret().as_bytes(),
    ))
}

pub fn build_authorization_url(
    request: OAuthAuthorizeUrlRequest<'_>,
) -> Result<OAuthAuthorizationUrl, AuthProductError> {
    let mut url = Url::parse(request.authorization_endpoint.as_str()).map_err(|_| {
        AuthProductError::invalid_request("oauth authorization endpoint must be an absolute url")
    })?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("client_id", request.client_id.as_str())
            .append_pair("redirect_uri", request.redirect_uri.as_str())
            .append_pair("response_type", "code")
            .append_pair("scope", &scope_text(request.scopes))
            .append_pair("state", request.state.as_str())
            .append_pair("code_challenge", request.code_challenge.as_str())
            .append_pair("code_challenge_method", "S256");
        for param in request.extra_params {
            pairs.append_pair(param.name(), param.value());
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
    let authorization_endpoint = OAuthAuthorizationEndpoint::new(GOOGLE_AUTHORIZATION_ENDPOINT)?;
    let client_id = OAuthClientId::new(client_id)?;
    let redirect_uri = OAuthRedirectUri::new(redirect_uri)?;
    let state = OAuthState::new(state)?;
    let mut extra_params = vec![
        OAuthExtraParam::new("access_type", "offline")?,
        OAuthExtraParam::new("prompt", "consent")?,
        OAuthExtraParam::new("include_granted_scopes", "true")?,
    ];
    if let Some(hosted_domain) = hosted_domain_hint {
        validate_authorize_fragment("google hosted domain", hosted_domain)?;
        extra_params.push(OAuthExtraParam::new("hd", hosted_domain)?);
    }
    build_authorization_url(OAuthAuthorizeUrlRequest {
        authorization_endpoint: &authorization_endpoint,
        client_id: &client_id,
        redirect_uri: &redirect_uri,
        state: &state,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_redirect_uri_rejects_non_loopback_http_and_non_url_values() {
        assert!(OAuthRedirectUri::new("http://example.com/callback").is_err());
        assert!(OAuthRedirectUri::new("not-a-url").is_err());
    }

    #[test]
    fn oauth_redirect_uri_accepts_https_and_loopback_http_values() {
        assert!(OAuthRedirectUri::new("https://example.com/callback").is_ok());
        assert!(OAuthRedirectUri::new("http://localhost:8080/callback").is_ok());
        assert!(OAuthRedirectUri::new("http://127.0.0.1:8080/callback").is_ok());
    }
}
