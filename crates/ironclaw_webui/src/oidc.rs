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
//!   live in whatever sign-in path the host binary owns (it can then
//!   mint a Reborn session via [`crate::signed_session_store`]).
//! - **Not** an audience-discovery layer — the host config names a
//!   fixed `audience` and `issuer`, and JWTs not matching both are
//!   rejected.
//!
//! Algorithm allowlist: RS256, RS384, RS512, ES256, ES384. Symmetric
//! algorithms are deliberately excluded — accepting HS256 with a
//! shared secret JWKS would let any party that knows the key forge
//! tokens.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{WebuiAuthentication, WebuiAuthenticator};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::UserId;
use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode, decode_header};
use parking_lot::RwLock;
use serde::Deserialize;
use thiserror::Error;
use url::{Host, Url};

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
    #[error(
        "JWKS URL `{url}` must use https:// — http transport lets an on-path attacker \
         replace the JWKS response and mint JWTs this authenticator would trust. \
         The only accepted exceptions are loopback hosts (localhost / 127.0.0.0/8 / ::1) \
         which are reachable only by the host process"
    )]
    InsecureJwksUrl { url: String },
    #[error(
        "JWKS URL `{url}` is malformed or not a usable https/http URL — refusing to \
         construct an OIDC authenticator with an unparseable JWKS endpoint"
    )]
    MalformedJwksUrl { url: String },
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
    #[serde(rename = "crv")]
    _crv: Option<String>,
}

#[derive(Debug, Default)]
struct JwksCache {
    fetched_at: Option<Instant>,
    keys: Vec<Jwk>,
    /// Timestamp of the last JWKS fetch failure. While within the
    /// failure-backoff window, `jwks()` returns the (possibly stale)
    /// cached keys instead of queueing another fetch that will likely
    /// also fail. Prevents request convoys behind a slow / unavailable
    /// JWKS endpoint.
    last_failure_at: Option<Instant>,
    /// Timestamp of the last *forced* (out-of-band) refresh that was
    /// triggered by an unknown-kid lookup. Tracked separately from
    /// `fetched_at` because TTL-driven refreshes happen on a slow
    /// cadence and shouldn't gate kid-rotation handling. Inside
    /// `JWKS_FORCED_REFRESH_MIN_INTERVAL`, an additional unknown-kid
    /// miss short-circuits without hitting the network — otherwise
    /// any caller with rotating fake `kid` values could DoS upstream
    /// (and us) before the per-caller rate limit kicks in. See
    /// `force_refresh_jwks` for the gate.
    last_forced_refresh_at: Option<Instant>,
}

/// How long to back off after a JWKS fetch failure before allowing
/// another network attempt. During this window, expired cache reads
/// return the stale keys (stale-while-revalidate); fresh cache hits
/// are unaffected.
const JWKS_FAILURE_BACKOFF: Duration = Duration::from_secs(30);

/// Minimum gap between successive *forced* JWKS refreshes (the ones
/// triggered by a token claiming a `kid` not in the current cache).
/// Without this, an unauthenticated caller streaming syntactically
/// valid JWTs whose `kid` values rotate every request can force one
/// upstream JWKS fetch per request — DoSing both this gateway and
/// the issuer before bearer auth even succeeds and before the
/// per-caller rate limiter has a caller key to attribute against.
/// Tokens that miss inside this window still 401 (the cached keys
/// don't match), but no network traffic is generated.
const JWKS_FORCED_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(60);

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
    /// Single-flight gate: when the JWKS cache expires, only one
    /// concurrent caller actually hits the upstream JWKS endpoint;
    /// other concurrent callers await the lock and pick up the
    /// freshly-cached keys without re-fetching. Without this, a burst
    /// of authenticated requests after cache TTL elapsed would
    /// stampede the JWKS endpoint.
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
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
        // Reject non-HTTPS JWKS URLs at construction so a misconfigured
        // deployment can't accidentally trust an attacker-replaceable
        // key set. Loopback is the documented exception — only the host
        // process itself can intercept those.
        require_secure_jwks_url(&config.jwks_url)?;
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
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            claim_to_user_id,
        })
    }

    /// Build a default claim-to-user-id mapper that uses the `sub`
    /// claim directly. Hosts that need richer policy supply their own
    /// closure to [`Self::new`].
    pub fn sub_claim_mapper() -> ClaimToUserIdFn {
        Arc::new(|claims: &IdTokenClaims| UserId::new(&claims.sub).ok())
    }

    fn try_cached_keys(&self) -> Option<Vec<Jwk>> {
        let guard = self.cache.read();
        if let Some(fetched_at) = guard.fetched_at
            && fetched_at.elapsed() < self.cache_ttl
            && !guard.keys.is_empty()
        {
            return Some(guard.keys.clone());
        }
        None
    }

    /// Stale-while-revalidate fallback: if the cache has keys at all,
    /// return them when we're within the failure-backoff window. The
    /// caller's request may still 401 on a rotated `kid`, but the
    /// gateway is not blocked behind another guaranteed-timeout fetch.
    fn try_stale_keys_during_backoff(&self) -> Option<Vec<Jwk>> {
        let guard = self.cache.read();
        let in_backoff = guard
            .last_failure_at
            .is_some_and(|at| at.elapsed() < JWKS_FAILURE_BACKOFF);
        if in_backoff && !guard.keys.is_empty() {
            return Some(guard.keys.clone());
        }
        None
    }

    async fn jwks(&self) -> Result<Vec<Jwk>, OidcAuthenticatorError> {
        // Cheap fast-path: if the cache is fresh, return it under the
        // read lock.
        if let Some(keys) = self.try_cached_keys() {
            return Ok(keys);
        }

        // Backoff path: a recent failure means the upstream JWKS
        // endpoint is currently flaky. Return stale keys without
        // queueing behind another fetch attempt. This is the convoy
        // break — without it, the first slow-failed fetch would
        // serialize every concurrent auth request behind one network
        // timeout at a time.
        if let Some(stale) = self.try_stale_keys_during_backoff() {
            return Ok(stale);
        }

        // Single-flight: only one concurrent caller actually hits the
        // upstream JWKS endpoint. Others await the lock and re-check
        // the cache — a fresh fetch may have populated it while they
        // were blocked.
        let _guard = self.refresh_lock.lock().await;
        if let Some(keys) = self.try_cached_keys() {
            return Ok(keys);
        }
        // Another caller may have just registered a failure that
        // started the backoff window while we waited on the lock.
        // Honor it without trying again.
        if let Some(stale) = self.try_stale_keys_during_backoff() {
            return Ok(stale);
        }
        self.refresh_jwks_locked().await
    }

    /// Force a JWKS refresh bypassing the cache TTL check. Used when
    /// a token's `kid` is not in the currently-cached keys — the
    /// normal signal that the issuer has rotated keys mid-TTL. Without
    /// this path, newly-signed tokens 401 for the full cache window.
    ///
    /// Two short-circuits guard against DoS / convoy patterns:
    ///
    /// 1. **Forced-refresh backoff** — if another forced refresh ran
    ///    within `JWKS_FORCED_REFRESH_MIN_INTERVAL`, return the
    ///    currently-cached keys without touching the network. Without
    ///    this, an unauthenticated caller streaming JWTs with rotating
    ///    `kid` values would force one upstream fetch per request,
    ///    DoSing the gateway and the issuer before bearer auth ever
    ///    succeeds (and before the per-caller rate limiter has a
    ///    caller key to attribute against).
    /// 2. **Single-flight** — if another caller refreshed while this
    ///    caller was waiting on `refresh_lock`, return that caller's
    ///    result without re-fetching. We detect "someone else
    ///    refreshed" by comparing the `fetched_at` timestamp before
    ///    and after taking the lock.
    ///
    /// A miss inside the backoff window still 401s the request
    /// (`decode_token` calls `lookup_jwk` again and finds nothing) —
    /// the security property of "unknown kid → reject" is preserved;
    /// only the upstream-fetch side effect is suppressed.
    async fn force_refresh_jwks(&self) -> Result<Vec<Jwk>, OidcAuthenticatorError> {
        let (before_fetched_at, before_forced_at) = {
            let guard = self.cache.read();
            (guard.fetched_at, guard.last_forced_refresh_at)
        };
        // Backoff short-circuit: if a previous forced refresh ran
        // within the min interval, return the current cache without
        // queueing another fetch.
        if let Some(last_forced) = before_forced_at
            && last_forced.elapsed() < JWKS_FORCED_REFRESH_MIN_INTERVAL
        {
            tracing::debug!(
                target = "ironclaw::reborn::webui_ingress::oidc",
                "forced JWKS refresh suppressed (within {}s backoff window)",
                JWKS_FORCED_REFRESH_MIN_INTERVAL.as_secs(),
            );
            return Ok(self.cache.read().keys.clone());
        }
        let _guard = self.refresh_lock.lock().await;
        // Re-check after acquiring the lock: another caller may have
        // run a forced refresh while we were blocked.
        let (after_fetched_at, after_forced_at) = {
            let guard = self.cache.read();
            (guard.fetched_at, guard.last_forced_refresh_at)
        };
        if before_fetched_at != after_fetched_at || before_forced_at != after_forced_at {
            return Ok(self.cache.read().keys.clone());
        }
        let result = self.refresh_jwks_locked().await;
        // Stamp the forced-refresh timestamp regardless of outcome —
        // a failed fetch should still cool the backoff so we don't
        // hammer a flaky upstream on every unknown-kid request.
        self.cache.write().last_forced_refresh_at = Some(Instant::now());
        result
    }

    /// Caller MUST hold `refresh_lock`. Performs the network fetch and
    /// replaces the cache atomically. On failure, stamps
    /// `last_failure_at` so subsequent callers within the backoff
    /// window can short-circuit through `try_stale_keys_during_backoff`
    /// instead of queueing behind another timeout. Also returns the
    /// (possibly stale) cached keys on failure if any exist —
    /// stale-while-revalidate keeps auth working through a transient
    /// JWKS outage; the caller's lookup may still 401 on a rotated
    /// kid, but the gateway doesn't fail every authenticated request.
    async fn refresh_jwks_locked(&self) -> Result<Vec<Jwk>, OidcAuthenticatorError> {
        let outcome = self.fetch_jwks().await;
        match outcome {
            Ok(keys) => {
                let mut guard = self.cache.write();
                guard.fetched_at = Some(Instant::now());
                guard.last_failure_at = None;
                guard.keys = keys.clone();
                Ok(keys)
            }
            Err(err) => {
                let stale = {
                    let mut guard = self.cache.write();
                    guard.last_failure_at = Some(Instant::now());
                    guard.keys.clone()
                };
                if !stale.is_empty() {
                    tracing::info!(
                        target = "ironclaw::reborn::webui_ingress::oidc",
                        error = %err,
                        "JWKS refresh failed; serving stale cached keys (backoff started)",
                    );
                    Ok(stale)
                } else {
                    Err(err)
                }
            }
        }
    }

    async fn fetch_jwks(&self) -> Result<Vec<Jwk>, OidcAuthenticatorError> {
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
        let mut jwk = lookup_jwk(&keys, kid.as_deref());
        // Key-rotation handling: if the token's `kid` isn't in the
        // cached JWKS, the issuer may have rotated keys mid-TTL.
        // Force one JWKS refresh and retry the lookup before rejecting
        // — without this, newly-signed tokens 401 for the full cache
        // window. Only retry when the token actually claimed a kid;
        // single-key issuers without `kid` don't benefit from a
        // refresh (the cached single key is either it or it isn't).
        if jwk.is_none() && kid.is_some() {
            tracing::debug!(
                target = "ironclaw::reborn::webui_ingress::oidc",
                "kid {kid:?} missing from cached JWKS; forcing refresh",
            );
            let refreshed = self.force_refresh_jwks().await?;
            jwk = lookup_jwk(&refreshed, kid.as_deref());
        }
        let Some(jwk) = jwk else {
            tracing::debug!(
                target = "ironclaw::reborn::webui_ingress::oidc",
                "JWKS has no key matching the token's kid even after refresh",
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

fn lookup_jwk(keys: &[Jwk], kid: Option<&str>) -> Option<Jwk> {
    match kid {
        Some(target) => keys
            .iter()
            .find(|key| key.kid.as_deref() == Some(target))
            .cloned(),
        None if keys.len() == 1 => keys.first().cloned(),
        None => None,
    }
}

/// Reject JWKS URLs that aren't HTTPS, with a narrow loopback
/// exception for local-dev / contract tests.
///
/// The guard is a real URL parser, not a string-prefix check, because
/// JWKS-URL trust is a security boundary: if it accepts an
/// attacker-controlled non-loopback plaintext endpoint, the OIDC
/// authenticator can be coerced into trusting forged signing keys.
/// Past attempts to substring-match a loopback prefix accepted
/// shapes like `http://127.evil.com/jwks`,
/// `http://127.0.0.1.evil.com/jwks`, and userinfo-disguised hosts
/// like `http://127.0.0.1@evil.com/jwks`.
///
/// Rules:
///
/// - The URL must parse with [`url::Url`] and have scheme `https` or
///   `http` — no `file:`, `ftp:`, `data:`, etc.
/// - No `userinfo` component (`user:pass@host`). `Url::host_str()`
///   would return the post-`@` host, but the authority looks like a
///   loopback host to a hand-rolled parser. Refuse rather than try
///   to disentangle.
/// - `https`: always accepted.
/// - `http`: accepted only if the host parses as an [`IpAddr`] whose
///   `is_loopback()` is true (covers 127.0.0.0/8 IPv4 and `::1`
///   IPv6), or equals the literal string `localhost`. Anything
///   else (including `127.evil.com`, `127.0.0.1.evil.com`,
///   `localhost.evil.com`) is rejected.
fn require_secure_jwks_url(url: &str) -> Result<(), OidcAuthenticatorError> {
    let parsed = Url::parse(url).map_err(|_| OidcAuthenticatorError::MalformedJwksUrl {
        url: url.to_string(),
    })?;

    // Block authority shapes that smuggle an attacker host past a
    // loopback-prefix check. `Url::username()` returns `""` when
    // absent; `Url::password()` returns `None`. Reject both.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(OidcAuthenticatorError::InsecureJwksUrl {
            url: url.to_string(),
        });
    }

    match parsed.scheme() {
        "https" => {
            // `host` is `None` for opaque/relative forms; treat that
            // as malformed so we never hand reqwest a JWKS URL we
            // cannot reason about.
            if parsed.host().is_none() {
                return Err(OidcAuthenticatorError::MalformedJwksUrl {
                    url: url.to_string(),
                });
            }
            Ok(())
        }
        "http" => match parsed.host() {
            None => Err(OidcAuthenticatorError::MalformedJwksUrl {
                url: url.to_string(),
            }),
            // `Host::Domain("localhost")` is accepted; anything else
            // domain-shaped (including `127.evil.com`,
            // `127.0.0.1.evil.com`, `localhost.evil.com`) gets
            // rejected because a DNS name resolves to whatever the
            // attacker controls, not to loopback.
            Some(Host::Domain("localhost")) => Ok(()),
            Some(Host::Domain(_)) => Err(OidcAuthenticatorError::InsecureJwksUrl {
                url: url.to_string(),
            }),
            // `Host::Ipv4` / `Host::Ipv6` give the parsed IP
            // directly; `is_loopback()` covers `127.0.0.0/8` and
            // `::1`.
            Some(Host::Ipv4(addr)) => check_loopback(IpAddr::V4(addr), url),
            Some(Host::Ipv6(addr)) => check_loopback(IpAddr::V6(addr), url),
        },
        _ => Err(OidcAuthenticatorError::MalformedJwksUrl {
            url: url.to_string(),
        }),
    }
}

fn check_loopback(addr: IpAddr, original: &str) -> Result<(), OidcAuthenticatorError> {
    if addr.is_loopback() {
        Ok(())
    } else {
        Err(OidcAuthenticatorError::InsecureJwksUrl {
            url: original.to_string(),
        })
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
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
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
        (self.claim_to_user_id)(&claims).map(WebuiAuthentication::user)
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

    #[test]
    fn require_secure_jwks_url_accepts_https() {
        assert!(require_secure_jwks_url("https://issuer.example/.well-known/jwks.json").is_ok());
        assert!(require_secure_jwks_url("https://issuer.example:8443/jwks").is_ok());
    }

    #[test]
    fn require_secure_jwks_url_accepts_loopback_http() {
        // Local-dev / contract tests run JWKS over loopback HTTP. All
        // of these must parse cleanly with `url::Url` and pass the
        // `IpAddr::is_loopback()` check (or be the literal
        // `localhost`).
        for url in [
            "http://localhost/jwks",
            "http://localhost:3000/jwks",
            "http://127.0.0.1:3000/jwks",
            "http://127.0.0.1/jwks",
            "http://127.42.0.1/jwks", // 127.0.0.0/8 — still loopback per RFC 6890
            "http://[::1]/jwks",
            "http://[::1]:3000/jwks",
        ] {
            assert!(
                require_secure_jwks_url(url).is_ok(),
                "loopback URL `{url}` should be accepted",
            );
        }
    }

    #[test]
    fn require_secure_jwks_url_rejects_plain_http_non_loopback() {
        for url in [
            "http://issuer.example/jwks",
            "http://192.168.1.10/jwks",
            "http://attacker.test/jwks",
            "http://10.0.0.1/jwks",
        ] {
            assert!(
                matches!(
                    require_secure_jwks_url(url),
                    Err(OidcAuthenticatorError::InsecureJwksUrl { .. })
                ),
                "non-https non-loopback URL `{url}` must be rejected, got: {:?}",
                require_secure_jwks_url(url),
            );
        }
    }

    // Regression for the JWKS loopback bypass — the previous
    // string-prefix check accepted any host whose ASCII prefix was
    // `127.`, so a DNS name like `127.evil.com` (which resolves to
    // an attacker-controlled IP) slipped through as "loopback".
    // The fix parses with `url::Url` and requires the host to
    // either equal `localhost` or parse as an `IpAddr` whose
    // `is_loopback()` is true; a DNS name cannot satisfy either
    // branch.
    #[test]
    fn require_secure_jwks_url_rejects_loopback_prefix_dns_bypass() {
        for url in [
            "http://127.evil.com/jwks",
            "http://127.0.0.1.evil.com/jwks",
            "http://127.0.0.1.attacker.example/jwks",
            "http://localhost.evil.com/jwks",
        ] {
            assert!(
                matches!(
                    require_secure_jwks_url(url),
                    Err(OidcAuthenticatorError::InsecureJwksUrl { .. })
                ),
                "DNS-shaped loopback-prefix host `{url}` must be rejected, got: {:?}",
                require_secure_jwks_url(url),
            );
        }
    }

    // Regression for the userinfo bypass — `http://127.0.0.1@evil.com`
    // looks like a loopback host to a substring matcher but the
    // real authority is `evil.com`. The fix refuses any URL that
    // carries a userinfo (username or password) component, on
    // either scheme, because a JWKS URL legitimately never needs
    // basic-auth credentials.
    #[test]
    fn require_secure_jwks_url_rejects_userinfo_authority() {
        for url in [
            "http://127.0.0.1@evil.com/jwks",
            "http://localhost@evil.com/jwks",
            "http://user:pass@127.0.0.1/jwks",
            "https://user@issuer.example/jwks",
            "https://user:pass@issuer.example/jwks",
        ] {
            assert!(
                matches!(
                    require_secure_jwks_url(url),
                    Err(OidcAuthenticatorError::InsecureJwksUrl { .. })
                ),
                "userinfo-bearing URL `{url}` must be rejected, got: {:?}",
                require_secure_jwks_url(url),
            );
        }
    }

    // Malformed inputs (empty string, unparseable, non-http(s)
    // schemes) must be refused outright rather than silently
    // bypassing the guard.
    #[test]
    fn require_secure_jwks_url_rejects_malformed_or_non_http_scheme() {
        for url in [
            "",
            "not a url",
            "http://",
            "ftp://issuer.example/jwks",
            "file:///etc/jwks.json",
            "data:application/json,{}",
        ] {
            assert!(
                require_secure_jwks_url(url).is_err(),
                "malformed/unsupported URL `{url}` must be rejected, got Ok",
            );
        }
    }

    #[test]
    fn lookup_jwk_finds_by_kid_and_falls_back_to_single_unkeyed() {
        let keys = vec![
            Jwk {
                kid: Some("key-1".into()),
                algorithm: None,
                kty: None,
                key_use: None,
                n: None,
                e: None,
                x: None,
                y: None,
                _crv: None,
            },
            Jwk {
                kid: Some("key-2".into()),
                algorithm: None,
                kty: None,
                key_use: None,
                n: None,
                e: None,
                x: None,
                y: None,
                _crv: None,
            },
        ];
        assert!(lookup_jwk(&keys, Some("key-2")).is_some());
        assert!(lookup_jwk(&keys, Some("key-3")).is_none());
        // A token without `kid` against a multi-key JWKS doesn't match.
        assert!(lookup_jwk(&keys, None).is_none());

        // Single-key JWKS without `kid` claim from the token → match.
        let single = vec![Jwk {
            kid: None,
            algorithm: None,
            kty: None,
            key_use: None,
            n: None,
            e: None,
            x: None,
            y: None,
            _crv: None,
        }];
        assert!(lookup_jwk(&single, None).is_some());
    }
}
