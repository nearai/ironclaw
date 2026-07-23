//! OAuth protocol helpers shared by the IronClaw auth engine and product-auth
//! services: PKCE challenge construction, validated OAuth value newtypes,
//! callback-state encode/decode, and redacted provider token projections.
//! Authorization URLs are constructed by the recipe-driven [`crate::AuthEngine`];
//! durable flow state and credential storage stay with the product-auth
//! services.

// v1 compatibility surface: the monolith's `src/auth/oauth.rs` re-exports the
// loopback callback transport under this historical path. The items live in
// [`crate::loopback_oauth`] (v1-only; see its header). Delete this re-export
// with v1.
pub use crate::loopback_oauth::{
    OAUTH_CALLBACK_PORT, OAuthCallbackError, bind_callback_listener, callback_host, callback_url,
    is_loopback_host, landing_html, wait_for_callback,
};

use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_host_api::ResourceScope;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    AuthProductError, AuthProductScope, AuthSessionId, AuthSurface, AuthorizationCodeHash,
    CredentialAccountLabel, OAuthAuthorizationCode, OpaqueStateHash, PkceVerifierHash,
    PkceVerifierSecret, ProviderScope, ids::AuthFlowId, validate_public_text,
};

/// IronClaw auth provider id for Google OAuth accounts.
pub const GOOGLE_PROVIDER_ID: &str = "google";

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

/// Wire descriptor for an OAuth callback state: the opaque-state prefix.
///
/// Distinct kinds get distinct prefixes so a state minted under one kind can
/// never decode under another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OAuthCallbackStateKind {
    prefix: &'static str,
}

impl OAuthCallbackStateKind {
    /// Recipe-engine callback state (`icr1.` prefix). Scope validation happens
    /// against the vendor recipe's scope ceiling when the engine prepares the
    /// flow, so decode applies no static scope policy.
    pub const RECIPE: Self = Self { prefix: "icr1." };
}

#[derive(Serialize, Deserialize)]
struct OAuthCallbackStateWire {
    flow_id: AuthFlowId,
    resource: ResourceScope,
    session_id: Option<AuthSessionId>,
    account_label: CredentialAccountLabel,
    requested_scopes: Vec<ProviderScope>,
    nonce: String,
}

/// Provider-agnostic host-resolved data carried through a static OAuth callback
/// URL in `state`.
///
/// The value is not authority by itself. Callback handlers must still hash the
/// full raw state and let `AuthFlowManager` compare it against the durable flow
/// before any provider exchange or completion side effect. The provider is
/// selected by an [`OAuthCallbackStateKind`], which fixes the wire prefix and
/// the scope-validation policy — so Google and Slack (and any future provider)
/// share one encode/decode implementation instead of a mirror per provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthCallbackState {
    kind: OAuthCallbackStateKind,
    flow_id: AuthFlowId,
    scope: AuthProductScope,
    account_label: CredentialAccountLabel,
    requested_scopes: Vec<ProviderScope>,
    nonce: String,
}

impl OAuthCallbackState {
    pub fn new(
        kind: OAuthCallbackStateKind,
        flow_id: AuthFlowId,
        scope: AuthProductScope,
        account_label: CredentialAccountLabel,
        requested_scopes: Vec<ProviderScope>,
    ) -> Result<Self, AuthProductError> {
        Ok(Self {
            kind,
            flow_id,
            scope,
            account_label,
            requested_scopes,
            nonce: ironclaw_common::pkce::generate_code_verifier(),
        })
    }

    pub fn flow_id(&self) -> AuthFlowId {
        self.flow_id
    }

    pub fn scope(&self) -> &AuthProductScope {
        &self.scope
    }

    pub fn account_label(&self) -> &CredentialAccountLabel {
        &self.account_label
    }

    pub fn requested_scopes(&self) -> &[ProviderScope] {
        &self.requested_scopes
    }

    pub fn encode(&self) -> Result<OAuthState, AuthProductError> {
        let wire = OAuthCallbackStateWire {
            flow_id: self.flow_id,
            resource: self.scope.resource.clone(),
            session_id: self.scope.session_id.clone(),
            account_label: self.account_label.clone(),
            requested_scopes: self.requested_scopes.clone(),
            nonce: self.nonce.clone(),
        };
        let payload =
            serde_json::to_vec(&wire).map_err(|_| AuthProductError::BackendUnavailable)?;
        OAuthState::new(format!(
            "{}{}",
            self.kind.prefix,
            URL_SAFE_NO_PAD.encode(payload)
        ))
    }

    pub fn decode(kind: OAuthCallbackStateKind, raw: &str) -> Result<Self, AuthProductError> {
        let encoded = raw
            .strip_prefix(kind.prefix)
            .ok_or(AuthProductError::MalformedCallback)?;
        let payload = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| AuthProductError::MalformedCallback)?;
        let wire: OAuthCallbackStateWire =
            serde_json::from_slice(&payload).map_err(|_| AuthProductError::MalformedCallback)?;
        validate_authorize_fragment("oauth nonce", &wire.nonce)
            .map_err(|_| AuthProductError::MalformedCallback)?;
        let mut scope = AuthProductScope::new(wire.resource, AuthSurface::Callback);
        if let Some(session_id) = wire.session_id {
            scope = scope.with_session_id(session_id);
        }
        Ok(Self {
            kind,
            flow_id: wire.flow_id,
            scope,
            account_label: wire.account_label,
            requested_scopes: wire.requested_scopes,
            nonce: wire.nonce,
        })
    }
}

impl fmt::Display for PkceCodeChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
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

/// Redacted token-response projection after provider exchange. It can be used
/// by provider clients before converting token material into secret handles.
#[derive(Clone)]
pub struct OAuthTokenResponse {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub scopes: Vec<ProviderScope>,
    pub expires_in_seconds: Option<u64>,
    pub provider_identity: Option<OAuthProviderIdentity>,
}

/// Non-secret provider identity fields returned by an OAuth token exchange.
///
/// Providers use different names for the same concept (`sub`, Slack
/// `authed_user.id`, app/team context, etc.). This redacted shape intentionally
/// carries only stable identifiers needed by host-owned binding logic; it never
/// stores raw provider response bodies or token material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthProviderIdentity {
    pub subject: OAuthProviderIdentitySubject,
    pub team_id: Option<String>,
    pub enterprise_id: Option<String>,
    pub app_id: Option<String>,
}

impl OAuthProviderIdentity {
    pub fn new(
        subject: impl Into<String>,
        team_id: Option<String>,
        enterprise_id: Option<String>,
        app_id: Option<String>,
    ) -> Result<Self, AuthProductError> {
        Ok(Self {
            subject: OAuthProviderIdentitySubject::new(subject)?,
            team_id: validate_optional_identity_field("oauth provider team id", team_id)?,
            enterprise_id: validate_optional_identity_field(
                "oauth provider enterprise id",
                enterprise_id,
            )?,
            app_id: validate_optional_identity_field("oauth provider app id", app_id)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct OAuthProviderIdentitySubject(String);

impl OAuthProviderIdentitySubject {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthProductError> {
        validate_public_text(value, "oauth provider subject", 256).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for OAuthProviderIdentitySubject {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for OAuthProviderIdentitySubject {
    type Error = AuthProductError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
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
            .field("provider_identity", &self.provider_identity)
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
            provider_identity: None,
        })
    }

    pub fn with_provider_identity(mut self, identity: OAuthProviderIdentity) -> Self {
        self.provider_identity = Some(identity);
        self
    }
}

fn validate_optional_identity_field(
    label: &'static str,
    value: Option<String>,
) -> Result<Option<String>, AuthProductError> {
    value
        .map(|value| validate_public_text(value, label, 256))
        .transpose()
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

    #[test]
    fn oauth_callback_state_round_trips_under_the_recipe_prefix() {
        let resource = ResourceScope {
            tenant_id: ironclaw_host_api::TenantId::new("tenant-a").unwrap(),
            user_id: ironclaw_host_api::UserId::new("user-a").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let scope = AuthProductScope::new(resource, AuthSurface::Callback);
        let label = CredentialAccountLabel::new("acct").unwrap();
        let flow_id = AuthFlowId::new();

        let scopes = vec![ProviderScope::new("search:read").unwrap()];
        let encoded = OAuthCallbackState::new(
            OAuthCallbackStateKind::RECIPE,
            flow_id,
            scope.clone(),
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

        // A value without the recipe prefix must not decode.
        assert!(
            OAuthCallbackState::decode(OAuthCallbackStateKind::RECIPE, "icg1.whatever").is_err()
        );
    }
}
