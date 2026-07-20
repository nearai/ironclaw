//! OAuth protocol helpers shared by Reborn product auth providers.
//!
//! This module intentionally owns only protocol-level pieces: Google OAuth
//! constants, PKCE challenge construction, authorization URL assembly, and
//! redacted provider token projections. Durable flow state, callback routing,
//! provider exchange, and credential storage remain owned by the product auth
//! services in this crate.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ironclaw_host_api::ResourceScope;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    AuthProductError, AuthProductScope, AuthSessionId, AuthSurface, AuthorizationCodeHash,
    CredentialAccountLabel, OAuthAuthorizationCode, OAuthAuthorizationUrl, OpaqueStateHash,
    PkceVerifierHash, PkceVerifierSecret, ProviderScope, ids::AuthFlowId, validate_public_text,
};

// Legacy v1 loopback OAuth transport, folded from the former `ironclaw_oauth`
// crate in W2.1. Reborn product auth must continue to use hosted durable
// callback routes instead of this fixed-port listener.
pub use crate::loopback_oauth::{
    OAUTH_CALLBACK_PORT, OAuthCallbackError, bind_callback_listener, callback_host, callback_url,
    is_loopback_host, landing_html, wait_for_callback,
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
/// Read-only access to Google Drive files.
pub const GOOGLE_DRIVE_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/drive.readonly";
/// Read/write access to Google Drive files.
pub const GOOGLE_DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive";
/// Read-only access to Google Docs documents.
pub const GOOGLE_DOCS_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/documents.readonly";
/// Read/write access to Google Docs documents.
pub const GOOGLE_DOCS_SCOPE: &str = "https://www.googleapis.com/auth/documents";
/// Read-only access to Google Sheets spreadsheets.
pub const GOOGLE_SHEETS_READONLY_SCOPE: &str =
    "https://www.googleapis.com/auth/spreadsheets.readonly";
/// Read/write access to Google Sheets spreadsheets.
pub const GOOGLE_SHEETS_SCOPE: &str = "https://www.googleapis.com/auth/spreadsheets";
/// Read-only access to Google Slides presentations.
pub const GOOGLE_SLIDES_READONLY_SCOPE: &str =
    "https://www.googleapis.com/auth/presentations.readonly";
/// Read/write access to Google Slides presentations.
pub const GOOGLE_SLIDES_SCOPE: &str = "https://www.googleapis.com/auth/presentations";
/// Read-only access to Gmail messages and metadata.
pub const GOOGLE_GMAIL_READONLY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
/// Permission to send Gmail messages.
pub const GOOGLE_GMAIL_SEND_SCOPE: &str = "https://www.googleapis.com/auth/gmail.send";
/// Permission to modify Gmail messages and drafts.
pub const GOOGLE_GMAIL_MODIFY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";

/// Reborn auth provider id for Slack personal (user-token) OAuth accounts.
///
/// Deliberately distinct from the bot Slack extension (`slack`) so a user
/// token can never collide with the workspace bot token.
pub const SLACK_PERSONAL_PROVIDER_ID: &str = "slack_personal";
/// Slack OAuth v2 authorization endpoint (user-token consent).
pub const SLACK_PERSONAL_AUTHORIZATION_ENDPOINT: &str = "https://slack.com/oauth/v2/authorize";
/// Slack OAuth v2 token endpoint (`oauth.v2.access`).
pub const SLACK_PERSONAL_TOKEN_ENDPOINT: &str = "https://slack.com/api/oauth.v2.access";

/// URL-safe S256 PKCE code challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkceCodeChallenge(String);

impl PkceCodeChallenge {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Google product-auth OAuth route configuration.
#[derive(Clone)]
pub struct GoogleOAuthRouteConfig {
    client_id: OAuthClientId,
    redirect_uri: OAuthRedirectUri,
    hosted_domain_hint: Option<String>,
}

impl GoogleOAuthRouteConfig {
    pub fn new(
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Result<Self, AuthProductError> {
        Ok(Self {
            client_id: OAuthClientId::new(client_id)?,
            redirect_uri: OAuthRedirectUri::new(redirect_uri)?,
            hosted_domain_hint: None,
        })
    }

    pub fn with_hosted_domain_hint(
        mut self,
        hosted_domain: impl Into<String>,
    ) -> Result<Self, AuthProductError> {
        let hosted_domain = hosted_domain.into();
        validate_authorize_fragment("google hosted domain hint", &hosted_domain)?;
        self.hosted_domain_hint = Some(hosted_domain);
        Ok(self)
    }

    pub fn client_id(&self) -> &OAuthClientId {
        &self.client_id
    }

    pub fn redirect_uri(&self) -> &OAuthRedirectUri {
        &self.redirect_uri
    }

    pub fn hosted_domain_hint(&self) -> Option<&str> {
        self.hosted_domain_hint.as_deref()
    }
}

impl fmt::Debug for GoogleOAuthRouteConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoogleOAuthRouteConfig")
            .field("client_id", &self.client_id.as_str())
            .field("redirect_uri", &self.redirect_uri)
            .field(
                "hosted_domain_hint",
                &self.hosted_domain_hint.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Scope-validation policy applied to an OAuth callback state's requested and
/// provider-returned scopes.
///
/// Providers differ in whether requested scopes are constrained to an allowlist
/// and whether the provider echoes granted scopes on the redirect. Capturing
/// that difference here lets one [`OAuthCallbackState`] type serve every
/// provider instead of a field-for-field mirror per provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OAuthCallbackScopePolicy {
    /// Requested and callback scopes must all be in the approved Google set
    /// (`is_allowed_google_scope`).
    GoogleAllowlist,
    /// Provider-defined open scope set (e.g. Slack user scopes); accepted as-is.
    ProviderDefined,
}

impl OAuthCallbackScopePolicy {
    /// Validate scopes supplied when *constructing* a new callback state.
    fn validate_requested(self, scopes: &[ProviderScope]) -> Result<(), AuthProductError> {
        match self {
            Self::GoogleAllowlist => validate_google_requested_scope_values(scopes),
            Self::ProviderDefined => Ok(()),
        }
    }

    /// Validate/normalize scopes recovered when *decoding* an encoded state.
    fn validate_callback(
        self,
        scopes: Vec<ProviderScope>,
    ) -> Result<Vec<ProviderScope>, AuthProductError> {
        match self {
            Self::GoogleAllowlist => validate_google_callback_scope_values(scopes),
            Self::ProviderDefined => Ok(scopes),
        }
    }
}

/// Provider descriptor for an OAuth callback state: the opaque-state wire prefix
/// plus the scope-validation policy.
///
/// Distinct providers get distinct prefixes so a state minted for one provider
/// can never decode under another. This is the one piece that differs between
/// providers; everything else about callback-state encode/decode is shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OAuthCallbackStateKind {
    prefix: &'static str,
    scope_policy: OAuthCallbackScopePolicy,
}

impl OAuthCallbackStateKind {
    /// Google product-auth callback state (`icg1.` prefix, Google scope allowlist).
    pub const GOOGLE: Self = Self {
        prefix: "icg1.",
        scope_policy: OAuthCallbackScopePolicy::GoogleAllowlist,
    };
    /// Slack personal (user-token) callback state (`ics1.` prefix, open scope set).
    pub const SLACK_PERSONAL: Self = Self {
        prefix: "ics1.",
        scope_policy: OAuthCallbackScopePolicy::ProviderDefined,
    };
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
        kind.scope_policy.validate_requested(&requested_scopes)?;
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
            requested_scopes: kind.scope_policy.validate_callback(wire.requested_scopes)?,
            nonce: wire.nonce,
        })
    }
}

/// Google product-auth OAuth callback state.
///
/// Thin back-compat facade over [`OAuthCallbackState`] pinned to
/// [`OAuthCallbackStateKind::GOOGLE`]. Production composition constructs and
/// decodes [`OAuthCallbackState`] directly; this named wrapper is retained
/// because the `auth_product_contract` integration test pins the Google-specific
/// constructor + scope-allowlist behavior through this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleOAuthCallbackState(OAuthCallbackState);

impl GoogleOAuthCallbackState {
    pub fn new(
        flow_id: AuthFlowId,
        scope: AuthProductScope,
        account_label: CredentialAccountLabel,
        requested_scopes: Vec<ProviderScope>,
    ) -> Result<Self, AuthProductError> {
        OAuthCallbackState::new(
            OAuthCallbackStateKind::GOOGLE,
            flow_id,
            scope,
            account_label,
            requested_scopes,
        )
        .map(Self)
    }

    pub fn flow_id(&self) -> AuthFlowId {
        self.0.flow_id()
    }

    pub fn scope(&self) -> &AuthProductScope {
        self.0.scope()
    }

    pub fn account_label(&self) -> &CredentialAccountLabel {
        self.0.account_label()
    }

    pub fn requested_scopes(&self) -> &[ProviderScope] {
        self.0.requested_scopes()
    }

    pub fn encode(&self) -> Result<OAuthState, AuthProductError> {
        self.0.encode()
    }

    pub fn decode(raw: &str) -> Result<Self, AuthProductError> {
        OAuthCallbackState::decode(OAuthCallbackStateKind::GOOGLE, raw).map(Self)
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

/// Selects which authorization query parameter carries the requested scopes.
///
/// Almost every OAuth 2.0 provider uses the standard `scope` parameter. Slack's
/// v2 user-token consent flow instead reads the requested scopes from
/// `user_scope` (its `scope` parameter is reserved for bot tokens). This
/// selector lets the one generic [`build_authorization_url_with_scope_param`]
/// builder serve both without a provider-specific URL assembler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthScopeParam {
    /// Standard OAuth 2.0 `scope` parameter (Google, Notion, DCR, ...).
    Scope,
    /// Slack v2 `user_scope` parameter (user-token consent).
    UserScope,
}

impl OAuthScopeParam {
    fn param_name(self) -> &'static str {
        match self {
            Self::Scope => "scope",
            Self::UserScope => "user_scope",
        }
    }
}

/// Builds a standard-`scope` authorization URL. Equivalent to
/// [`build_authorization_url_with_scope_param`] with [`OAuthScopeParam::Scope`].
pub fn build_authorization_url(
    request: OAuthAuthorizeUrlRequest<'_>,
) -> Result<OAuthAuthorizationUrl, AuthProductError> {
    build_authorization_url_with_scope_param(request, OAuthScopeParam::Scope)
}

/// Builds an authorization URL, placing the requested scopes in the query
/// parameter chosen by `scope_param`.
///
/// The core OAuth parameters (`client_id`, `redirect_uri`, `response_type`,
/// scope, `state`, PKCE challenge) plus any provider `extra_params` are assembled
/// identically regardless of the scope parameter name; only the scope key
/// differs. This replaces the former hand-rolled Slack authorization-URL builder.
pub fn build_authorization_url_with_scope_param(
    request: OAuthAuthorizeUrlRequest<'_>,
    scope_param: OAuthScopeParam,
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
            .append_pair(scope_param.param_name(), &scope_text(request.scopes))
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

pub fn parse_google_requested_scopes(
    raw_scopes: &[String],
) -> Result<Vec<ProviderScope>, AuthProductError> {
    let scopes = raw_scopes
        .iter()
        .map(|scope| ProviderScope::new(scope.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    validate_google_requested_scope_values(&scopes)?;
    Ok(scopes)
}

pub fn parse_google_callback_scopes(
    raw: Option<&str>,
) -> Result<Option<Vec<ProviderScope>>, AuthProductError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.trim() != raw {
        return Err(AuthProductError::MalformedCallback);
    }
    if raw.is_empty() {
        return Ok(Some(Vec::new()));
    }
    raw.split([' ', ','])
        .filter(|scope| !scope.is_empty())
        .map(|scope| {
            ProviderScope::new(scope.to_string()).map_err(|_| AuthProductError::MalformedCallback)
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub fn is_allowed_google_scope(scope: &str) -> bool {
    matches!(
        scope,
        GOOGLE_CALENDAR_READONLY_SCOPE
            | GOOGLE_CALENDAR_EVENTS_SCOPE
            | GOOGLE_DRIVE_READONLY_SCOPE
            | GOOGLE_DRIVE_SCOPE
            | GOOGLE_DOCS_READONLY_SCOPE
            | GOOGLE_DOCS_SCOPE
            | GOOGLE_SHEETS_READONLY_SCOPE
            | GOOGLE_SHEETS_SCOPE
            | GOOGLE_SLIDES_READONLY_SCOPE
            | GOOGLE_SLIDES_SCOPE
            | GOOGLE_GMAIL_READONLY_SCOPE
            | GOOGLE_GMAIL_SEND_SCOPE
            | GOOGLE_GMAIL_MODIFY_SCOPE
    )
}

fn validate_google_requested_scope_values(
    scopes: &[ProviderScope],
) -> Result<(), AuthProductError> {
    if scopes.is_empty() {
        return Err(AuthProductError::invalid_request(
            "google oauth scopes must not be empty",
        ));
    }
    if scopes
        .iter()
        .any(|scope| !is_allowed_google_scope(scope.as_str()))
    {
        return Err(AuthProductError::invalid_request(
            "google oauth scope is not allowed",
        ));
    }
    Ok(())
}

fn validate_google_callback_scope_values(
    scopes: Vec<ProviderScope>,
) -> Result<Vec<ProviderScope>, AuthProductError> {
    if scopes.is_empty() {
        return Err(AuthProductError::MalformedCallback);
    }
    if scopes
        .iter()
        .any(|scope| !is_allowed_google_scope(scope.as_str()))
    {
        return Err(AuthProductError::MalformedCallback);
    }
    Ok(scopes)
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

    #[test]
    fn unified_authorization_url_builder_selects_scope_param_per_provider() {
        // Equivalence: one builder emits Slack's `user_scope` and Google's
        // standard `scope` (plus Google offline-consent extras) depending only on
        // the `OAuthScopeParam` selector — no provider-specific URL assembler.
        let verifier = PkceVerifierSecret::new(SecretString::from(
            ironclaw_common::pkce::generate_code_verifier(),
        ))
        .unwrap();
        let challenge = pkce_s256_challenge(&verifier);

        // Slack personal: `user_scope`, no bot `scope`, no Google extras.
        let slack_endpoint =
            OAuthAuthorizationEndpoint::new(SLACK_PERSONAL_AUTHORIZATION_ENDPOINT).unwrap();
        let slack_client = OAuthClientId::new("slack-client-id").unwrap();
        let slack_redirect = OAuthRedirectUri::new(
            "http://127.0.0.1:3000/api/reborn/product-auth/oauth/slack_personal/callback",
        )
        .unwrap();
        let slack_state = OAuthState::new("teststatevalue").unwrap();
        let slack_scopes = vec![
            ProviderScope::new("search:read").unwrap(),
            ProviderScope::new("users:read").unwrap(),
        ];
        let slack_url = build_authorization_url_with_scope_param(
            OAuthAuthorizeUrlRequest {
                authorization_endpoint: &slack_endpoint,
                client_id: &slack_client,
                redirect_uri: &slack_redirect,
                state: &slack_state,
                code_challenge: &challenge,
                scopes: &slack_scopes,
                extra_params: &[],
            },
            OAuthScopeParam::UserScope,
        )
        .unwrap();
        let slack_parsed = Url::parse(slack_url.as_str()).unwrap();
        assert!(
            slack_parsed
                .as_str()
                .starts_with("https://slack.com/oauth/v2/authorize")
        );
        let slack_pairs: std::collections::HashMap<String, String> =
            slack_parsed.query_pairs().into_owned().collect();
        assert_eq!(
            slack_pairs.get("user_scope").map(String::as_str),
            Some("search:read users:read")
        );
        assert!(
            !slack_pairs.contains_key("scope"),
            "Slack personal flow must not request bot `scope`"
        );
        assert_eq!(
            slack_pairs.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );

        // Google: standard `scope`, no `user_scope`, offline-consent extras.
        let google_scopes = vec![ProviderScope::new(GOOGLE_CALENDAR_READONLY_SCOPE).unwrap()];
        let google_url = build_google_authorization_url(
            "google-client",
            "https://app.example/callback",
            "teststatevalue",
            &challenge,
            &google_scopes,
            None,
        )
        .unwrap();
        let google_pairs: std::collections::HashMap<String, String> =
            Url::parse(google_url.as_str())
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect();
        assert_eq!(
            google_pairs.get("scope").map(String::as_str),
            Some(GOOGLE_CALENDAR_READONLY_SCOPE)
        );
        assert!(!google_pairs.contains_key("user_scope"));
        assert_eq!(
            google_pairs.get("access_type").map(String::as_str),
            Some("offline")
        );
    }

    #[test]
    fn oauth_callback_state_round_trips_per_provider_prefix() {
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

        // Slack: open scope set, `ics1.` prefix.
        let slack_scopes = vec![ProviderScope::new("search:read").unwrap()];
        let slack_encoded = OAuthCallbackState::new(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            flow_id,
            scope.clone(),
            label.clone(),
            slack_scopes.clone(),
        )
        .unwrap()
        .encode()
        .unwrap();
        assert!(slack_encoded.as_str().starts_with("ics1."));
        let slack_decoded = OAuthCallbackState::decode(
            OAuthCallbackStateKind::SLACK_PERSONAL,
            slack_encoded.as_str(),
        )
        .unwrap();
        assert_eq!(slack_decoded.flow_id(), flow_id);
        assert_eq!(slack_decoded.account_label(), &label);
        assert_eq!(slack_decoded.requested_scopes(), slack_scopes.as_slice());
        // A Slack-minted state must not decode under the Google prefix.
        assert!(
            OAuthCallbackState::decode(OAuthCallbackStateKind::GOOGLE, slack_encoded.as_str())
                .is_err()
        );

        // Google: allowlisted scopes, `icg1.` prefix; matches the Google facade.
        let google_scopes = vec![ProviderScope::new(GOOGLE_CALENDAR_READONLY_SCOPE).unwrap()];
        let google_encoded = OAuthCallbackState::new(
            OAuthCallbackStateKind::GOOGLE,
            flow_id,
            scope.clone(),
            label.clone(),
            google_scopes.clone(),
        )
        .unwrap()
        .encode()
        .unwrap();
        assert!(google_encoded.as_str().starts_with("icg1."));
        let google_decoded =
            OAuthCallbackState::decode(OAuthCallbackStateKind::GOOGLE, google_encoded.as_str())
                .unwrap();
        assert_eq!(google_decoded.requested_scopes(), google_scopes.as_slice());
        assert_eq!(
            GoogleOAuthCallbackState::decode(google_encoded.as_str())
                .unwrap()
                .requested_scopes(),
            google_scopes.as_slice()
        );

        // Google policy rejects non-allowlisted requested scopes; Slack accepts.
        assert!(
            OAuthCallbackState::new(
                OAuthCallbackStateKind::GOOGLE,
                flow_id,
                scope.clone(),
                label.clone(),
                vec![ProviderScope::new("https://www.googleapis.com/auth/gmail.insert").unwrap()],
            )
            .is_err()
        );
        assert!(
            OAuthCallbackState::new(
                OAuthCallbackStateKind::SLACK_PERSONAL,
                flow_id,
                scope,
                label,
                vec![ProviderScope::new("admin").unwrap()],
            )
            .is_ok()
        );
    }

    #[test]
    fn google_oauth_allowlist_includes_gsuite_wasm_scopes() {
        for scope in [
            GOOGLE_DRIVE_READONLY_SCOPE,
            GOOGLE_DRIVE_SCOPE,
            GOOGLE_DOCS_READONLY_SCOPE,
            GOOGLE_DOCS_SCOPE,
            GOOGLE_SHEETS_READONLY_SCOPE,
            GOOGLE_SHEETS_SCOPE,
            GOOGLE_SLIDES_READONLY_SCOPE,
            GOOGLE_SLIDES_SCOPE,
        ] {
            assert!(is_allowed_google_scope(scope), "{scope} must be allowed");
            assert!(parse_google_requested_scopes(&[scope.to_string()]).is_ok());
        }
    }

    #[test]
    fn google_callback_scope_parser_accepts_provider_returned_extras() {
        let scopes = parse_google_callback_scopes(Some(&format!(
            "openid email profile {GOOGLE_GMAIL_READONLY_SCOPE}"
        )))
        .expect("callback scopes")
        .expect("present callback scopes");

        assert_eq!(
            scopes,
            vec![
                ProviderScope::new("openid").unwrap(),
                ProviderScope::new("email").unwrap(),
                ProviderScope::new("profile").unwrap(),
                ProviderScope::new(GOOGLE_GMAIL_READONLY_SCOPE).unwrap(),
            ]
        );
    }
}
