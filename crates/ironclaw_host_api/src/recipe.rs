//! Auth and ingress-verification **recipe** vocabulary.
//!
//! Recipes are pure data an extension manifest declares and the host
//! executes: the auth engine implements each auth *method* once
//! (`oauth2_code`, `api_key`) and vendors differ only in parameters; the
//! ingress verifier executes signature recipes so signing secrets never
//! reach an adapter. There is deliberately no auth adapter trait — see
//! `docs/reborn/extension-runtime/overview.md` §4.3.
//!
//! Everything here is declaration vocabulary: validation and serialization
//! only, no execution.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{HostApiError, SecretHandle};

/// OAuth authorize-request parameters the host constructs itself. A recipe's
/// `extra_authorize_params` may never name them (directly or via the
/// resolved scope parameter) — recipes carry vendor quirks, not protocol
/// control.
pub const RESERVED_AUTHORIZE_PARAMS: &[&str] = &[
    "state",
    "redirect_uri",
    "code_challenge",
    "code_challenge_method",
    "client_id",
    "client_secret",
    "response_type",
    "code",
    "scope",
];

/// Maximum reference-token depth of a [`BoundedJsonPointer`].
pub const MAX_JSON_POINTER_DEPTH: usize = 8;

/// An RFC 6901 JSON pointer restricted for recipe use: non-empty, at most
/// [`MAX_JSON_POINTER_DEPTH`] reference tokens, no empty tokens, and no `*`
/// (recipes select exactly one value — there is no wildcard matching).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BoundedJsonPointer(String);

impl BoundedJsonPointer {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Unescaped reference tokens, in order.
    pub fn tokens(&self) -> Vec<String> {
        self.0
            .split('/')
            .skip(1)
            .map(|token| token.replace("~1", "/").replace("~0", "~"))
            .collect()
    }

    fn validate(value: &str) -> Result<(), HostApiError> {
        let invalid = |reason: &str| HostApiError::InvalidId {
            kind: "json_pointer",
            value: value.to_string(),
            reason: reason.to_string(),
        };
        if value.is_empty() {
            return Err(invalid("must not be empty"));
        }
        if !value.starts_with('/') {
            return Err(invalid("must start with '/'"));
        }
        let tokens: Vec<&str> = value.split('/').skip(1).collect();
        if tokens.len() > MAX_JSON_POINTER_DEPTH {
            return Err(invalid("exceeds maximum depth of 8 reference tokens"));
        }
        for token in tokens {
            if token.is_empty() {
                return Err(invalid("must not contain empty reference tokens"));
            }
            if token.contains('*') {
                return Err(invalid("wildcards are not supported"));
            }
            // RFC 6901: `~` is only valid as `~0` or `~1`.
            let mut chars = token.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '~' && !matches!(chars.peek(), Some('0') | Some('1')) {
                    return Err(invalid("invalid '~' escape (only ~0 and ~1 are defined)"));
                }
            }
        }
        Ok(())
    }
}

impl serde::Serialize for BoundedJsonPointer {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for BoundedJsonPointer {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// An endpoint URL a recipe may call: `https` only, literal host (no
/// wildcards), no userinfo, no fragment.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HttpsEndpoint(String);

impl HttpsEndpoint {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The endpoint's host (lowercase).
    pub fn host(&self) -> String {
        let rest = self.0.split("://").nth(1).unwrap_or_default();
        let host_port = rest.split(['/', '?']).next().unwrap_or_default();
        host_port
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    fn validate(value: &str) -> Result<(), HostApiError> {
        let invalid = |reason: &str| HostApiError::InvalidId {
            kind: "https_endpoint",
            value: value.to_string(),
            reason: reason.to_string(),
        };
        let Some(rest) = value.strip_prefix("https://") else {
            return Err(invalid("must use the https scheme"));
        };
        let authority_and_path = rest;
        let authority = authority_and_path
            .split(['/', '?'])
            .next()
            .unwrap_or_default();
        if authority.is_empty() {
            return Err(invalid("must have a host"));
        }
        if authority.contains('@') {
            return Err(invalid("must not contain userinfo"));
        }
        if authority.contains('*') {
            return Err(invalid("host must be literal (no wildcards)"));
        }
        if value.contains('#') {
            return Err(invalid("must not contain a fragment"));
        }
        if value.chars().any(char::is_whitespace) {
            return Err(invalid("must not contain whitespace"));
        }
        Ok(())
    }
}

impl serde::Serialize for HttpsEndpoint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for HttpsEndpoint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for HttpsEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One vendor's auth recipe (`[auth.<vendor>]` in a v3 manifest). The tag is
/// the auth *method* the host engine implements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum VendorAuthRecipe {
    Oauth2Code(Box<OAuth2CodeRecipe>),
    ApiKey(ApiKeyRecipe),
}

impl VendorAuthRecipe {
    pub fn display_name(&self) -> &str {
        match self {
            Self::Oauth2Code(recipe) => &recipe.display_name,
            Self::ApiKey(recipe) => &recipe.display_name,
        }
    }

    /// Scope ceiling this recipe grants (empty for `api_key`).
    pub fn scope_ceiling(&self) -> &[String] {
        match self {
            Self::Oauth2Code(recipe) => &recipe.scopes,
            Self::ApiKey(_) => &[],
        }
    }

    /// Semantic validation beyond what deserialization enforces.
    pub fn validate(&self) -> Result<(), RecipeValidationError> {
        match self {
            Self::Oauth2Code(recipe) => recipe.validate(),
            Self::ApiKey(recipe) => recipe.validate(),
        }
    }

    /// The declared idle-keepalive threshold, if any (`oauth2_code` only —
    /// `api_key` credentials have no refresh token to keep alive).
    pub fn keepalive_idle_threshold(&self) -> Option<std::time::Duration> {
        match self {
            Self::Oauth2Code(recipe) => recipe
                .refresh
                .as_ref()
                .and_then(|refresh| refresh.keepalive_idle_seconds)
                .map(|seconds| std::time::Duration::from_secs(u64::from(seconds))),
            Self::ApiKey(_) => None,
        }
    }

    /// Whether two recipes for a shared vendor are compatible: identical
    /// except `scopes` and `display_name`
    /// (`docs/reborn/extension-runtime/overview.md` §3.2).
    pub fn compatible_for_shared_vendor(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Oauth2Code(a), Self::Oauth2Code(b)) => {
                let mut a = a.clone();
                let mut b = b.clone();
                a.scopes = Vec::new();
                b.scopes = Vec::new();
                a.display_name = String::new();
                b.display_name = String::new();
                a == b
            }
            (Self::ApiKey(a), Self::ApiKey(b)) => {
                let mut a = a.clone();
                let mut b = b.clone();
                a.display_name = String::new();
                b.display_name = String::new();
                a == b
            }
            _ => false,
        }
    }
}

/// OAuth 2.0 authorization-code recipe. The engine owns `state`,
/// `redirect_uri`, PKCE, `client_id`, `response_type`, and the scope
/// parameter; the recipe carries endpoints, parameter names, and response
/// field paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuth2CodeRecipe {
    pub display_name: String,
    pub authorization_endpoint: HttpsEndpoint,
    pub token_endpoint: HttpsEndpoint,
    /// Authorize/scope parameter name; defaults to `scope` (some vendors
    /// reserve `scope=` for another grant type and name a dedicated
    /// user-scope parameter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_param: Option<String>,
    /// Scope list separator; defaults to a space.
    #[serde(default)]
    pub scope_join: ScopeJoin,
    /// PKCE mode; defaults to S256. `none` must be declared explicitly.
    #[serde(default)]
    pub pkce: PkceMode,
    /// The recipe's scope ceiling: requested scopes must intersect into it.
    pub scopes: Vec<String>,
    /// Extra vendor-specific authorize parameters. Reserved protocol
    /// parameters are rejected by [`OAuth2CodeRecipe::validate`].
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_authorize_params: BTreeMap<String, String>,
    /// Deployment-level client-credential handles. Absent means the vendor
    /// requires dynamic client registration (RFC 7591 — generic hosted-MCP
    /// behavior, implemented once by the host auth engine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_credentials: Option<RecipeClientCredentials>,
    /// How client credentials are presented during token exchange.
    #[serde(default)]
    pub exchange_auth: TokenExchangeAuth,
    pub token_response: TokenResponseMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<IdentityRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<RefreshRecipe>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoke: Option<RevokeRecipe>,
}

impl OAuth2CodeRecipe {
    /// The effective scope parameter name.
    pub fn scope_param(&self) -> &str {
        self.scope_param.as_deref().unwrap_or("scope")
    }

    pub fn validate(&self) -> Result<(), RecipeValidationError> {
        if self.display_name.trim().is_empty() {
            return Err(RecipeValidationError::EmptyDisplayName);
        }
        if self.scopes.iter().any(|scope| scope.trim().is_empty()) {
            return Err(RecipeValidationError::EmptyScope);
        }
        if let Some(param) = &self.scope_param
            && param.trim().is_empty()
        {
            return Err(RecipeValidationError::EmptyScopeParam);
        }
        let scope_param = self.scope_param().to_string();
        for key in self.extra_authorize_params.keys() {
            if RESERVED_AUTHORIZE_PARAMS.contains(&key.as_str()) || *key == scope_param {
                return Err(RecipeValidationError::ReservedAuthorizeParam { param: key.clone() });
            }
        }
        if let Some(seconds) = self.refresh.as_ref().and_then(|r| r.keepalive_idle_seconds)
            && !(MIN_KEEPALIVE_IDLE_SECONDS..=MAX_KEEPALIVE_IDLE_SECONDS).contains(&seconds)
        {
            return Err(RecipeValidationError::KeepaliveIdleOutOfRange { seconds });
        }
        Ok(())
    }
}

/// Scope list separator on the authorize request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeJoin {
    #[default]
    Space,
    Comma,
}

impl ScopeJoin {
    pub fn separator(&self) -> &'static str {
        match self {
            Self::Space => " ",
            Self::Comma => ",",
        }
    }
}

/// PKCE mode for the authorization-code flow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PkceMode {
    #[default]
    S256,
    None,
}

/// Deployment-level client-credential handles resolved through the secret
/// store — never inline values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeClientCredentials {
    pub client_id_handle: SecretHandle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret_handle: Option<SecretHandle>,
}

/// Client-credential presentation on the token-exchange request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenExchangeAuth {
    #[default]
    PostBody,
    Basic,
}

/// Where token-response fields live (bounded JSON pointers).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenResponseMap {
    pub access_token: BoundedJsonPointer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<BoundedJsonPointer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<BoundedJsonPointer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeExtraction>,
}

/// How the granted scope is read from the token response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeExtraction {
    pub path: BoundedJsonPointer,
    #[serde(default)]
    pub missing: MissingScopeBehavior,
}

/// What a missing granted-scope field means.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingScopeBehavior {
    /// Fail the exchange: the vendor should have reported granted scopes.
    #[default]
    Reject,
    /// Treat the requested scopes as granted.
    FallbackToRequested,
}

/// Identity claim extraction: from the token response, or from a follow-up
/// endpoint called with the fresh credential. Claims beyond `account_id`
/// (e.g. `team_id`) are free-form named pointers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRecipe {
    /// When set, claims are read from this endpoint's response instead of
    /// the token response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<IdentityEndpoint>,
    pub account_id: BoundedJsonPointer,
    // NOTE: no deny_unknown_fields here on purpose — extra keys are named
    // identity claims (serde flatten and deny_unknown_fields are mutually
    // exclusive).
    #[serde(flatten)]
    pub claims: BTreeMap<String, BoundedJsonPointer>,
}

/// A follow-up identity endpoint (`GET` with the fresh credential injected
/// by the engine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityEndpoint {
    pub url: HttpsEndpoint,
}

/// Lower bound for [`RefreshRecipe::keepalive_idle_seconds`] (one hour).
pub const MIN_KEEPALIVE_IDLE_SECONDS: u32 = 3_600;
/// Upper bound for [`RefreshRecipe::keepalive_idle_seconds`] (365 days).
pub const MAX_KEEPALIVE_IDLE_SECONDS: u32 = 31_536_000;

/// Refresh-token semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshRecipe {
    /// Whether the vendor rotates the refresh token on every refresh.
    #[serde(default)]
    pub rotates_refresh_token: bool,
    /// Vendor lifetime constraint: refresh tokens die after this many seconds
    /// of inactivity (some vendors expire idle refresh tokens — e.g. for apps
    /// in a "testing" publishing status — after a fixed window). Declaring it
    /// opts the vendor's accounts into the host auth engine's proactive
    /// keepalive sweep; absent means the vendor's tokens do not idle-expire
    /// and are never swept.
    /// Bounded to [`MIN_KEEPALIVE_IDLE_SECONDS`, `MAX_KEEPALIVE_IDLE_SECONDS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_idle_seconds: Option<u32>,
}

/// Best-effort remote revocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeRecipe {
    pub endpoint: HttpsEndpoint,
    /// Form parameter carrying the token; defaults to `token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_param: Option<String>,
}

/// Personal-access-token recipe: render fields from data, store, optionally
/// validate with a probe. Same account state machine as OAuth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyRecipe {
    pub display_name: String,
    pub fields: Vec<RecipeSecretField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ApiKeyValidationProbe>,
}

impl ApiKeyRecipe {
    pub fn validate(&self) -> Result<(), RecipeValidationError> {
        if self.display_name.trim().is_empty() {
            return Err(RecipeValidationError::EmptyDisplayName);
        }
        if self.fields.is_empty() {
            return Err(RecipeValidationError::ApiKeyWithoutFields);
        }
        if let Some(probe) = &self.validation {
            if probe.success_status.is_empty() {
                return Err(RecipeValidationError::ProbeWithoutSuccessStatus);
            }
            if !self
                .fields
                .iter()
                .any(|field| field.handle == probe.inject.handle)
            {
                return Err(RecipeValidationError::ProbeInjectsUndeclaredHandle {
                    handle: probe.inject.handle.as_str().to_string(),
                });
            }
        }
        Ok(())
    }
}

/// One operator-entered secret field (shared shape with administrator configuration
/// fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeSecretField {
    pub handle: SecretHandle,
    pub label: String,
    #[serde(default = "default_true")]
    pub secret: bool,
}

fn default_true() -> bool {
    true
}

/// The optional post-store validation probe for an `api_key` recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyValidationProbe {
    #[serde(default)]
    pub method: ProbeMethod,
    pub url: HttpsEndpoint,
    pub success_status: Vec<u16>,
    pub inject: ProbeInjection,
}

/// Probe HTTP method (read-only probes only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ProbeMethod {
    #[default]
    Get,
}

/// Which stored field the probe injects and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeInjection {
    pub handle: SecretHandle,
    #[serde(flatten)]
    pub target: crate::RuntimeCredentialTarget,
}

/// Semantic recipe validation failures (path context is added by the
/// manifest parser).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecipeValidationError {
    #[error("display_name must not be empty")]
    EmptyDisplayName,
    #[error("scopes must not contain empty entries")]
    EmptyScope,
    #[error("scope_param must not be empty")]
    EmptyScopeParam,
    #[error(
        "extra_authorize_params must not name the reserved protocol parameter `{param}` \
         (the host constructs it)"
    )]
    ReservedAuthorizeParam { param: String },
    #[error("api_key recipes must declare at least one field")]
    ApiKeyWithoutFields,
    #[error("validation probe must declare at least one success status")]
    ProbeWithoutSuccessStatus,
    #[error("validation probe injects `{handle}`, which is not one of the recipe's fields")]
    ProbeInjectsUndeclaredHandle { handle: String },
    #[error(
        "refresh.keepalive_idle_seconds must be between {MIN_KEEPALIVE_IDLE_SECONDS} and \
         {MAX_KEEPALIVE_IDLE_SECONDS} seconds (got {seconds})"
    )]
    KeepaliveIdleOutOfRange { seconds: u32 },
    #[error("signed_payload must not be empty")]
    EmptySignedPayload,
    #[error("signed_payload `body` segments must be `body = true`")]
    SignedPayloadBodyFalse,
    #[error("timestamp verification requires both timestamp_header and max_age_seconds")]
    IncompleteTimestampRule,
}

/// Declarative inbound-request verification the host executes before an
/// adapter sees anything (`[channel.ingress.verification]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IngressVerificationRecipe {
    HmacSha256(HmacSha256VerificationRecipe),
    SharedSecretHeader(SharedSecretHeaderRecipe),
    /// No verification (explicit; e.g. a vendor with IP allowlisting only).
    None,
}

impl IngressVerificationRecipe {
    pub fn validate(&self) -> Result<(), RecipeValidationError> {
        match self {
            Self::HmacSha256(recipe) => recipe.validate(),
            Self::SharedSecretHeader(_) | Self::None => Ok(()),
        }
    }

    /// The secret handle this recipe verifies with, if any.
    pub fn secret_handle(&self) -> Option<&SecretHandle> {
        match self {
            Self::HmacSha256(recipe) => Some(&recipe.secret_handle),
            Self::SharedSecretHeader(recipe) => Some(&recipe.secret_handle),
            Self::None => None,
        }
    }
}

/// HMAC-SHA256 signature verification over a declared byte construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HmacSha256VerificationRecipe {
    pub secret_handle: SecretHandle,
    pub signature_header: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_prefix: Option<String>,
    #[serde(default)]
    pub signature_encoding: SignatureEncoding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_seconds: Option<u32>,
    pub signed_payload: Vec<SignedPayloadSegment>,
}

impl HmacSha256VerificationRecipe {
    pub fn validate(&self) -> Result<(), RecipeValidationError> {
        if self.signed_payload.is_empty() {
            return Err(RecipeValidationError::EmptySignedPayload);
        }
        for segment in &self.signed_payload {
            if let SignedPayloadSegment::Body { body } = segment
                && !body
            {
                return Err(RecipeValidationError::SignedPayloadBodyFalse);
            }
        }
        if self.timestamp_header.is_some() != self.max_age_seconds.is_some() {
            return Err(RecipeValidationError::IncompleteTimestampRule);
        }
        Ok(())
    }
}

/// Signature byte encoding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureEncoding {
    #[default]
    Hex,
    Base64,
}

/// One segment of the signed-payload byte construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SignedPayloadSegment {
    Literal { literal: String },
    Header { header: String },
    Body { body: bool },
}

/// Constant-time shared-secret header comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedSecretHeaderRecipe {
    pub secret_handle: SecretHandle,
    pub header: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slack_shaped_recipe_toml() -> &'static str {
        r#"
method = "oauth2_code"
display_name = "Example account"
authorization_endpoint = "https://vendor.example/oauth/v2/authorize"
token_endpoint = "https://vendor.example/api/oauth.v2.access"
scope_param = "user_scope"
pkce = "s256"
scopes = ["search:read", "chat:write"]
client_credentials = { client_id_handle = "vendor_oauth_client_id", client_secret_handle = "vendor_oauth_client_secret" }

[token_response]
access_token = "/authed_user/access_token"
scope = { path = "/authed_user/scope", missing = "fallback_to_requested" }

[identity]
account_id = "/authed_user/id"
team_id = "/team/id"
"#
    }

    #[test]
    fn oauth2_recipe_parses_the_documented_shape() {
        let recipe: VendorAuthRecipe = toml::from_str(slack_shaped_recipe_toml()).unwrap();
        recipe.validate().unwrap();
        let VendorAuthRecipe::Oauth2Code(recipe) = &recipe else {
            panic!("expected oauth2_code");
        };
        assert_eq!(recipe.scope_param(), "user_scope");
        assert_eq!(recipe.pkce, PkceMode::S256);
        assert_eq!(recipe.exchange_auth, TokenExchangeAuth::PostBody);
        let identity = recipe.identity.as_ref().unwrap();
        assert_eq!(identity.account_id.as_str(), "/authed_user/id");
        assert_eq!(
            identity
                .claims
                .get("team_id")
                .map(BoundedJsonPointer::as_str),
            Some("/team/id")
        );
        assert_eq!(
            recipe.token_response.scope.as_ref().unwrap().missing,
            MissingScopeBehavior::FallbackToRequested
        );
    }

    #[test]
    fn oauth2_recipe_rejects_reserved_authorize_params() {
        for reserved in ["state", "redirect_uri", "code_challenge", "user_scope"] {
            let mut recipe: VendorAuthRecipe = toml::from_str(slack_shaped_recipe_toml()).unwrap();
            if let VendorAuthRecipe::Oauth2Code(inner) = &mut recipe {
                inner
                    .extra_authorize_params
                    .insert(reserved.to_string(), "x".to_string());
            }
            let error = recipe.validate().unwrap_err();
            assert!(
                matches!(error, RecipeValidationError::ReservedAuthorizeParam { ref param } if param == reserved),
                "expected reserved-param rejection for {reserved}, got {error:?}"
            );
        }
    }

    #[test]
    fn non_https_endpoints_fail_at_deserialize() {
        let toml = slack_shaped_recipe_toml().replace(
            "https://vendor.example/oauth",
            "http://vendor.example/oauth",
        );
        let error = toml::from_str::<VendorAuthRecipe>(&toml).unwrap_err();
        assert!(error.to_string().contains("https"), "{error}");
    }

    #[test]
    fn json_pointers_are_bounded_and_wildcard_free() {
        assert!(BoundedJsonPointer::new("/a/b").is_ok());
        assert!(BoundedJsonPointer::new("/a~0b/c~1d").is_ok());
        assert!(BoundedJsonPointer::new("").is_err());
        assert!(BoundedJsonPointer::new("a/b").is_err());
        assert!(BoundedJsonPointer::new("/a//b").is_err());
        assert!(BoundedJsonPointer::new("/a/*").is_err());
        assert!(BoundedJsonPointer::new("/a/~2").is_err());
        assert!(BoundedJsonPointer::new("/1/2/3/4/5/6/7/8/9").is_err());
        assert_eq!(
            BoundedJsonPointer::new("/authed_user/access_token")
                .unwrap()
                .tokens(),
            vec!["authed_user", "access_token"]
        );
    }

    #[test]
    fn unknown_recipe_fields_fail_closed() {
        // Top-level unknown key (inserted before the first sub-table so TOML
        // attributes it to the recipe itself, not the identity claims map).
        let toml = slack_shaped_recipe_toml().replace(
            "display_name = \"Example account\"",
            "display_name = \"Example account\"\nsurprise = \"x\"",
        );
        let error = toml::from_str::<VendorAuthRecipe>(&toml).unwrap_err();
        assert!(error.to_string().contains("surprise"), "{error}");
    }

    #[test]
    fn api_key_recipe_parses_and_validates_probe_handles() {
        let toml = r#"
method = "api_key"
display_name = "Example personal access token"
fields = [ { handle = "example_token", label = "Personal access token", secret = true } ]
validation = { method = "GET", url = "https://api.vendor.example/user", success_status = [200], inject = { handle = "example_token", type = "header", name = "authorization", prefix = "Bearer " } }
"#;
        let recipe: VendorAuthRecipe = toml::from_str(toml).unwrap();
        recipe.validate().unwrap();

        let broken = toml.replace(
            "inject = { handle = \"example_token\"",
            "inject = { handle = \"other_token\"",
        );
        let recipe: VendorAuthRecipe = toml::from_str(&broken).unwrap();
        assert!(matches!(
            recipe.validate().unwrap_err(),
            RecipeValidationError::ProbeInjectsUndeclaredHandle { .. }
        ));
    }

    #[test]
    fn hmac_verification_recipe_parses_the_documented_shape() {
        let toml = r#"
kind = "hmac_sha256"
secret_handle = "vendor_signing_secret"
signature_header = "X-Vendor-Signature"
signature_prefix = "v0="
signature_encoding = "hex"
timestamp_header = "X-Vendor-Request-Timestamp"
max_age_seconds = 300
signed_payload = [
  { literal = "v0:" },
  { header = "X-Vendor-Request-Timestamp" },
  { literal = ":" },
  { body = true },
]
"#;
        let recipe: IngressVerificationRecipe = toml::from_str(toml).unwrap();
        recipe.validate().unwrap();
        let IngressVerificationRecipe::HmacSha256(recipe) = &recipe else {
            panic!("expected hmac_sha256");
        };
        assert_eq!(recipe.signed_payload.len(), 4);
        assert!(matches!(
            recipe.signed_payload[3],
            SignedPayloadSegment::Body { body: true }
        ));
    }

    #[test]
    fn hmac_verification_recipe_rejects_incomplete_timestamp_rule() {
        let toml = r#"
kind = "hmac_sha256"
secret_handle = "vendor_signing_secret"
signature_header = "X-Vendor-Signature"
timestamp_header = "X-Vendor-Request-Timestamp"
signed_payload = [ { body = true } ]
"#;
        let recipe: IngressVerificationRecipe = toml::from_str(toml).unwrap();
        assert!(matches!(
            recipe.validate().unwrap_err(),
            RecipeValidationError::IncompleteTimestampRule
        ));
    }

    #[test]
    fn shared_vendor_compatibility_ignores_scopes_and_display_name_only() {
        let base: VendorAuthRecipe = toml::from_str(slack_shaped_recipe_toml()).unwrap();
        let mut other = base.clone();
        if let VendorAuthRecipe::Oauth2Code(inner) = &mut other {
            inner.scopes = vec!["different:scope".to_string()];
            inner.display_name = "Other".to_string();
        }
        assert!(base.compatible_for_shared_vendor(&other));

        let mut conflicting = base.clone();
        if let VendorAuthRecipe::Oauth2Code(inner) = &mut conflicting {
            inner.token_endpoint =
                HttpsEndpoint::new("https://vendor.example/other/token").unwrap();
        }
        assert!(!base.compatible_for_shared_vendor(&conflicting));

        let mut keepalive_conflicting = base.clone();
        if let VendorAuthRecipe::Oauth2Code(inner) = &mut keepalive_conflicting {
            inner.refresh = Some(RefreshRecipe {
                rotates_refresh_token: false,
                keepalive_idle_seconds: Some(604_800),
            });
        }
        assert!(
            !base.compatible_for_shared_vendor(&keepalive_conflicting),
            "a keepalive-threshold difference is a shared-vendor conflict"
        );
    }

    #[test]
    fn refresh_keepalive_idle_threshold_parses_validates_and_projects() {
        let toml = format!(
            "{}\n[refresh]\nkeepalive_idle_seconds = 604800\n",
            slack_shaped_recipe_toml()
        );
        let recipe: VendorAuthRecipe = toml::from_str(&toml).unwrap();
        recipe.validate().unwrap();
        assert_eq!(
            recipe.keepalive_idle_threshold(),
            Some(std::time::Duration::from_secs(604_800)),
            "a declared keepalive threshold projects as a duration"
        );

        let bare: VendorAuthRecipe = toml::from_str(slack_shaped_recipe_toml()).unwrap();
        assert_eq!(
            bare.keepalive_idle_threshold(),
            None,
            "vendors that do not declare the threshold are never swept"
        );
    }

    #[test]
    fn refresh_keepalive_idle_seconds_out_of_range_fails_closed() {
        for out_of_range in [
            MIN_KEEPALIVE_IDLE_SECONDS - 1,
            MAX_KEEPALIVE_IDLE_SECONDS + 1,
        ] {
            let toml = format!(
                "{}\n[refresh]\nkeepalive_idle_seconds = {out_of_range}\n",
                slack_shaped_recipe_toml()
            );
            let recipe: VendorAuthRecipe = toml::from_str(&toml).unwrap();
            assert!(
                matches!(
                    recipe.validate().unwrap_err(),
                    RecipeValidationError::KeepaliveIdleOutOfRange { .. }
                ),
                "keepalive_idle_seconds = {out_of_range} must fail closed"
            );
        }
    }

    #[test]
    fn recipe_wire_shape_round_trips() {
        let recipe: VendorAuthRecipe = toml::from_str(slack_shaped_recipe_toml()).unwrap();
        let json = serde_json::to_string(&recipe).unwrap();
        let back: VendorAuthRecipe = serde_json::from_str(&json).unwrap();
        assert_eq!(recipe, back);
    }
}
