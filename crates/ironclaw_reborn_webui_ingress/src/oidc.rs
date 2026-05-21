//! OIDC bearer-token authenticator for the Reborn WebChat v2 gateway.
//!
//! The browser presents an OIDC ID-token (JWT) as the Authorization
//! header; the host verifies it against the issuer's published JWKS,
//! validates `iss` / `aud` / `exp` / `nbf`, then maps the `sub` claim
//! to a `UserId` through a host-supplied closure.
//!
//! What this module is NOT:
//!
//! - **Not** a full OIDC client — there is no authorization-code
//!   exchange, no PKCE, no token endpoint, no refresh handling. Those
//!   live in whatever sign-in path the host binary owns (it then
//!   typically mints a Reborn session via the `SessionStore` from
//!   [`crate::session`]).
//! - **Not** an audience-discovery layer — the host config names a
//!   fixed `audience` and `issuer`, and JWTs not matching both are
//!   rejected.
//!
//! Algorithm allowlist: RS256, RS384, RS512, ES256, ES384. Symmetric
//! algorithms are deliberately excluded — accepting HS256 with a
//! shared secret JWKS would let any party that knows the key forge
//! tokens.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::UserId;
use ironclaw_reborn_composition::WebuiAuthenticator;
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode, decode_header};
use parking_lot::RwLock;
use serde::Deserialize;
use thiserror::Error;

/// Configuration for [`OidcAuthenticator`].
#[derive(Debug, Clone)]
pub struct OidcAuthenticatorConfig {
    /// Required `iss` claim. JWTs claiming a different issuer are
    /// rejected before signature verification.
    pub issuer: String,
    /// Required `aud` claim. Multi-audience JWTs match if any entry
    /// equals this value.
    pub audience: String,
    /// JWKS URL (typically `<issuer>/.well-known/jwks.json`).
    pub jwks_url: String,
    /// JWKS cache TTL. Each lookup checks if the cache has expired
    /// and refreshes lazily. Defaults to 5 minutes if `None`.
    pub jwks_cache_ttl: Option<Duration>,
    /// HTTP timeout for JWKS fetches. Defaults to 5 seconds.
    pub http_timeout: Option<Duration>,
}

impl OidcAuthenticatorConfig {
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        jwks_url: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            audience: audience.into(),
            jwks_url: jwks_url.into(),
            jwks_cache_ttl: None,
            http_timeout: None,
        }
    }
}

/// Mapper from a verified ID-token claim set to a Reborn `UserId`.
///
/// Host installations have policy on how OIDC subjects map to users
/// (sub claim, email, custom claim, tenant-prefixed sub, …). This
/// closure is host-owned so the authenticator stays policy-free.
pub type ClaimToUserIdFn = Arc<dyn Fn(&IdTokenClaims) -> Option<UserId> + Send + Sync + 'static>;

/// Minimal projection of OIDC standard claims this authenticator
/// inspects. Extra claims pass through to the host mapper via
/// [`IdTokenClaims::extra`].
#[derive(Debug, Clone, Deserialize)]
pub struct IdTokenClaims {
    pub iss: String,
    pub sub: String,
    pub aud: AudienceClaim,
    pub exp: i64,
    #[serde(default)]
    pub nbf: Option<i64>,
    #[serde(default)]
    pub iat: Option<i64>,
    #[serde(default, flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Accept both single-string and array-of-string `aud` shapes
/// (RFC 7519 allows either).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AudienceClaim {
    Single(String),
    Multi(Vec<String>),
}

impl AudienceClaim {
    pub fn contains(&self, expected: &str) -> bool {
        match self {
            Self::Single(value) => value == expected,
            Self::Multi(values) => values.iter().any(|v| v == expected),
        }
    }
}

#[derive(Debug, Error)]
pub enum OidcAuthenticatorError {
    #[error("JWKS fetch failed: {0}")]
    JwksFetch(String),
    #[error("invalid JWKS payload: {0}")]
    JwksParse(String),
    #[error("HTTP client construction failed: {0}")]
    HttpClient(String),
}

/// JWKS document shape from the issuer's `.well-known/jwks.json`.
#[derive(Debug, Clone, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kid: Option<String>,
    #[serde(rename = "alg")]
    algorithm: Option<String>,
    kty: Option<String>,
    #[serde(rename = "use")]
    key_use: Option<String>,
    n: Option<String>,
    e: Option<String>,
    x: Option<String>,
    y: Option<String>,
    // `crv` (curve identifier) is parsed for completeness but not
    // load-bearing — `jsonwebtoken::DecodingKey::from_ec_components`
    // infers the curve from the algorithm. Kept in the struct so a
    // future audit can reject keys whose declared curve disagrees
    // with the token's algorithm.
    #[allow(dead_code)]
    crv: Option<String>,
}

#[derive(Debug, Default)]
struct JwksCache {
    fetched_at: Option<Instant>,
    keys: Vec<Jwk>,
}

/// OIDC ID-token authenticator. Cheap to clone — the JWKS cache is
/// `Arc`-shared.
#[derive(Clone)]
pub struct OidcAuthenticator {
    issuer: String,
    audience: String,
    jwks_url: String,
    cache_ttl: Duration,
    http: reqwest::Client,
    cache: Arc<RwLock<JwksCache>>,
    claim_to_user_id: ClaimToUserIdFn,
}

impl std::fmt::Debug for OidcAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcAuthenticator")
            .field("issuer", &self.issuer)
            .field("audience", &self.audience)
            .field("jwks_url", &self.jwks_url)
            .finish_non_exhaustive()
    }
}

impl OidcAuthenticator {
    pub fn new(
        config: OidcAuthenticatorConfig,
        claim_to_user_id: ClaimToUserIdFn,
    ) -> Result<Self, OidcAuthenticatorError> {
        let cache_ttl = config.jwks_cache_ttl.unwrap_or(Duration::from_secs(300));
        let http_timeout = config.http_timeout.unwrap_or(Duration::from_secs(5));
        let http = reqwest::Client::builder()
            .timeout(http_timeout)
            .build()
            .map_err(|err| OidcAuthenticatorError::HttpClient(err.to_string()))?;
        Ok(Self {
            issuer: config.issuer,
            audience: config.audience,
            jwks_url: config.jwks_url,
            cache_ttl,
            http,
            cache: Arc::new(RwLock::new(JwksCache::default())),
            claim_to_user_id,
        })
    }

    /// Build a default claim-to-user-id mapper that uses the `sub`
    /// claim directly. Hosts that need richer policy supply their own
    /// closure to [`Self::new`].
    pub fn sub_claim_mapper() -> ClaimToUserIdFn {
        Arc::new(|claims: &IdTokenClaims| UserId::new(&claims.sub).ok())
    }

    async fn jwks(&self) -> Result<Vec<Jwk>, OidcAuthenticatorError> {
        // Cheap fast-path: if the cache is fresh, return it under the
        // read lock.
        {
            let guard = self.cache.read();
            if let Some(fetched_at) = guard.fetched_at
                && fetched_at.elapsed() < self.cache_ttl
                && !guard.keys.is_empty()
            {
                return Ok(guard.keys.clone());
            }
        }

        // Slow-path: fetch + replace cache.
        let response = self
            .http
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|err| OidcAuthenticatorError::JwksFetch(err.to_string()))?;
        if !response.status().is_success() {
            return Err(OidcAuthenticatorError::JwksFetch(format!(
                "JWKS endpoint returned status {}",
                response.status()
            )));
        }
        let jwks: Jwks = response
            .json()
            .await
            .map_err(|err| OidcAuthenticatorError::JwksParse(err.to_string()))?;
        let mut guard = self.cache.write();
        guard.fetched_at = Some(Instant::now());
        guard.keys = jwks.keys.clone();
        Ok(jwks.keys)
    }

    async fn decode_token(
        &self,
        token: &str,
    ) -> Result<Option<TokenData<IdTokenClaims>>, OidcAuthenticatorError> {
        // Pre-validation: peek at the JWT header to pick the algorithm
        // and the kid the token claims to be signed with.
        let header = match decode_header(token) {
            Ok(header) => header,
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_ingress::oidc",
                    error = %error,
                    "JWT header decode failed",
                );
                return Ok(None);
            }
        };
        let algorithm = match header.alg {
            Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::ES256
            | Algorithm::ES384 => header.alg,
            _ => {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_ingress::oidc",
                    alg = ?header.alg,
                    "rejecting JWT signed with disallowed algorithm",
                );
                return Ok(None);
            }
        };
        let kid = header.kid;

        let keys = self.jwks().await?;
        let jwk = match &kid {
            Some(target) => keys
                .iter()
                .find(|key| key.kid.as_ref() == Some(target))
                .cloned(),
            None if keys.len() == 1 => keys.first().cloned(),
            None => None,
        };
        let Some(jwk) = jwk else {
            tracing::debug!(
                target = "ironclaw::reborn::webui_ingress::oidc",
                "JWKS has no key matching the token's kid",
            );
            return Ok(None);
        };
        let key = match build_decoding_key(&jwk, algorithm) {
            Some(key) => key,
            None => return Ok(None),
        };
        let mut validation = Validation::new(algorithm);
        validation.set_issuer(std::slice::from_ref(&self.issuer));
        validation.set_audience(std::slice::from_ref(&self.audience));
        // Defense in depth: don't accept multi-algorithm tokens.
        validation.algorithms = vec![algorithm];
        match decode::<IdTokenClaims>(token, &key, &validation) {
            Ok(data) => Ok(Some(data)),
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::webui_ingress::oidc",
                    error = %error,
                    "JWT verification failed",
                );
                Ok(None)
            }
        }
    }
}

fn build_decoding_key(jwk: &Jwk, algorithm: Algorithm) -> Option<DecodingKey> {
    // Reject keys whose declared algorithm disagrees with the token's
    // algorithm (rfc7517 "alg" parameter), and keys flagged for
    // non-signature use.
    if let Some(declared) = &jwk.algorithm
        && declared.parse::<Algorithm>().ok() != Some(algorithm)
    {
        return None;
    }
    if let Some(key_use) = &jwk.key_use
        && key_use != "sig"
    {
        return None;
    }
    match jwk.kty.as_deref() {
        Some("RSA") => {
            let (Some(n), Some(e)) = (jwk.n.as_ref(), jwk.e.as_ref()) else {
                return None;
            };
            DecodingKey::from_rsa_components(n, e).ok()
        }
        Some("EC") => {
            let (Some(x), Some(y)) = (jwk.x.as_ref(), jwk.y.as_ref()) else {
                return None;
            };
            DecodingKey::from_ec_components(x, y).ok()
        }
        _ => None,
    }
}

#[async_trait]
impl WebuiAuthenticator for OidcAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<UserId> {
        let token_data = match self.decode_token(token).await {
            Ok(Some(data)) => data,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::reborn::webui_ingress::oidc",
                    error = %error,
                    "OIDC JWKS unavailable; failing closed",
                );
                return None;
            }
        };
        let claims = token_data.claims;
        // Defense in depth: jsonwebtoken validates issuer/audience/exp
        // already, but verify our local view in case a future config
        // mistake widens validation.
        if claims.iss != self.issuer {
            return None;
        }
        if !claims.aud.contains(&self.audience) {
            return None;
        }
        let now = Utc::now();
        if expired(now, claims.exp) {
            return None;
        }
        if let Some(nbf) = claims.nbf
            && in_future(now, nbf)
        {
            return None;
        }
        (self.claim_to_user_id)(&claims)
    }
}

fn expired(now: DateTime<Utc>, exp: i64) -> bool {
    now.timestamp() >= exp
}

fn in_future(now: DateTime<Utc>, nbf: i64) -> bool {
    now.timestamp() < nbf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audience_claim_contains_handles_string_and_array() {
        let single = AudienceClaim::Single("aud-a".into());
        let multi = AudienceClaim::Multi(vec!["aud-x".into(), "aud-y".into()]);
        assert!(single.contains("aud-a"));
        assert!(!single.contains("aud-z"));
        assert!(multi.contains("aud-x"));
        assert!(multi.contains("aud-y"));
        assert!(!multi.contains("aud-z"));
    }

    #[test]
    fn expired_rejects_when_exp_is_in_past() {
        let now = DateTime::<Utc>::from_timestamp(1_000_000, 0).expect("ts");
        assert!(expired(now, 999_000));
        assert!(!expired(now, 1_000_001));
    }

    #[test]
    fn in_future_rejects_when_nbf_is_after_now() {
        let now = DateTime::<Utc>::from_timestamp(1_000_000, 0).expect("ts");
        assert!(in_future(now, 1_000_001));
        assert!(!in_future(now, 999_999));
    }

    #[test]
    fn sub_claim_mapper_builds_valid_user_id() {
        let mapper = OidcAuthenticator::sub_claim_mapper();
        let claims = IdTokenClaims {
            iss: "https://issuer.example".into(),
            sub: "alice".into(),
            aud: AudienceClaim::Single("aud".into()),
            exp: 9_000_000_000,
            nbf: None,
            iat: None,
            extra: HashMap::new(),
        };
        let user = mapper(&claims).expect("user resolves");
        assert_eq!(user.as_str(), "alice");
    }

    #[test]
    fn sub_claim_mapper_rejects_invalid_user_id_grammar() {
        let mapper = OidcAuthenticator::sub_claim_mapper();
        let claims = IdTokenClaims {
            iss: "https://issuer.example".into(),
            // `UserId::new` rejects path-separator-containing values
            // because the id is used in scoped filesystem paths.
            sub: "alice/admin".into(),
            aud: AudienceClaim::Single("aud".into()),
            exp: 9_000_000_000,
            nbf: None,
            iat: None,
            extra: HashMap::new(),
        };
        assert!(mapper(&claims).is_none());
    }
}
