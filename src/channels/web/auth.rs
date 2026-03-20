//! Authentication middleware for the web gateway.
//!
//! Supports two auth mechanisms, tried in order:
//!
//! ```text
//!   Request
//!     │
//!     ▼
//!   ┌─────────────────────────┐
//!   │ Authorization: Bearer …│──► token match ──► ALLOW
//!   └────────────┬────────────┘
//!                │ no match / missing
//!                ▼
//!   ┌─────────────────────────┐
//!   │ OIDC JWT header         │──► sig + claims OK ──► ALLOW
//!   │ (if configured)         │
//!   └────────────┬────────────┘
//!                │ no match / missing / disabled
//!                ▼
//!   ┌─────────────────────────┐
//!   │ ?token=xxx query param  │──► token match ──► ALLOW
//!   │ (SSE/WS endpoints only) │    (only GET on streaming paths)
//!   └────────────┬────────────┘
//!                │ no match
//!                ▼
//!              401 Unauthorized
//! ```
//!
//! **Bearer token** — constant-time comparison, `Bearer` prefix is
//! case-insensitive per RFC 6750 §2.1.
//!
//! **OIDC JWT** — enabled via `GATEWAY_OIDC_ENABLED=true`. The gateway
//! reads a JWT from a configurable header (default: `x-amzn-oidc-data`),
//! fetches the signing key from a JWKS endpoint, and verifies the
//! signature + claims. Designed for reverse-proxy setups like AWS ALB
//! with Okta/Cognito, but works with any RFC-compliant OIDC provider.
//!
//! **Query-string token** — only allowed on SSE/WS endpoints where
//! browser APIs cannot set custom headers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;

use crate::config::GatewayOidcConfig;

// ── Auth state shared via axum middleware ────────────────────────────────

/// Shared auth state injected via axum middleware state.
#[derive(Clone)]
pub struct AuthState {
    pub token: String,
    pub oidc: Option<OidcState>,
}

// ── OIDC types ──────────────────────────────────────────────────────────

/// Cached OIDC signing key with its resolved algorithm.
#[derive(Clone)]
struct CachedKey {
    decoding_key: DecodingKey,
    algorithm: Algorithm,
    fetched_at: Instant,
}

/// OIDC JWT authentication state.
///
/// Holds the configuration, an HTTP client for JWKS fetches, and a
/// per-`kid` key cache with 1-hour TTL.
#[derive(Clone)]
pub struct OidcState {
    config: GatewayOidcConfig,
    key_cache: Arc<RwLock<HashMap<String, CachedKey>>>,
    http_client: reqwest::Client,
}

/// OIDC-specific errors (internal, never shown to unauthenticated clients).
#[derive(Debug, thiserror::Error)]
enum OidcError {
    #[error("missing `kid` in JWT header")]
    MissingKid,
    #[error("unsupported algorithm: {0}")]
    UnsupportedAlgorithm(String),
    #[error("key fetch failed: {0}")]
    KeyFetch(String),
    #[error("signature verification failed")]
    InvalidSignature,
    #[error("claim validation failed: {0}")]
    InvalidClaims(String),
}

const KEY_CACHE_TTL: Duration = Duration::from_secs(3600);

impl OidcState {
    /// Build OIDC state from gateway config. Returns `None` if OIDC is not configured.
    pub fn from_config(oidc: &GatewayOidcConfig) -> Self {
        Self {
            config: oidc.clone(),
            key_cache: Arc::new(RwLock::new(HashMap::new())),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client must build"),
        }
    }

    /// Header name containing the JWT.
    fn header_name(&self) -> &str {
        &self.config.header
    }

    // ── Key fetching ────────────────────────────────────────────────────

    /// Fetch a PEM or JWK from an ALB-style per-key URL (`{kid}` placeholder).
    async fn fetch_single_key(
        &self,
        url: &str,
        alg: Algorithm,
    ) -> Result<DecodingKey, OidcError> {
        let body = self.fetch_url_text(url).await?;
        let trimmed = body.trim();

        if trimmed.starts_with("-----BEGIN") {
            // PEM-encoded public key (EC or RSA).
            match alg {
                Algorithm::ES256 | Algorithm::ES384 => {
                    DecodingKey::from_ec_pem(trimmed.as_bytes())
                        .map_err(|e| OidcError::KeyFetch(format!("EC PEM parse: {e}")))
                }
                _ => DecodingKey::from_rsa_pem(trimmed.as_bytes())
                    .map_err(|e| OidcError::KeyFetch(format!("RSA PEM parse: {e}"))),
            }
        } else {
            // Assume single JWK JSON object.
            let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_str(trimmed)
                .map_err(|e| OidcError::KeyFetch(format!("JWK parse: {e}")))?;
            DecodingKey::from_jwk(&jwk)
                .map_err(|e| OidcError::KeyFetch(format!("JWK decode: {e}")))
        }
    }

    /// Fetch from a standard JWKS endpoint and find the key matching `kid`.
    async fn fetch_jwks_key(
        &self,
        url: &str,
        kid: &str,
    ) -> Result<(DecodingKey, Algorithm), OidcError> {
        let body = self.fetch_url_text(url).await?;
        let jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_str(&body)
            .map_err(|e| OidcError::KeyFetch(format!("JWKS parse: {e}")))?;
        let jwk = jwks
            .find(kid)
            .ok_or_else(|| OidcError::KeyFetch(format!("kid '{kid}' not found in JWKS")))?;
        let alg = resolve_algorithm(jwk)?;
        let key = DecodingKey::from_jwk(jwk)
            .map_err(|e| OidcError::KeyFetch(format!("JWK decode: {e}")))?;
        Ok((key, alg))
    }

    /// HTTP GET helper with timeout and error status check.
    async fn fetch_url_text(&self, url: &str) -> Result<String, OidcError> {
        self.http_client
            .get(url)
            .send()
            .await
            .map_err(|e| OidcError::KeyFetch(format!("HTTP request to {url}: {e}")))?
            .error_for_status()
            .map_err(|e| OidcError::KeyFetch(format!("HTTP {url}: {e}")))?
            .text()
            .await
            .map_err(|e| OidcError::KeyFetch(format!("reading body from {url}: {e}")))
    }

    /// Get the signing key for `kid`, using cache when available (1h TTL).
    async fn get_or_fetch_key(
        &self,
        kid: &str,
        alg: Algorithm,
    ) -> Result<(DecodingKey, Algorithm), OidcError> {
        // Fast path: cache hit with valid TTL.
        {
            let cache = self.key_cache.read().await;
            if let Some(cached) = cache.get(kid)
                && cached.fetched_at.elapsed() < KEY_CACHE_TTL
            {
                return Ok((cached.decoding_key.clone(), cached.algorithm));
            }
        }

        // Slow path: fetch and cache.
        let (key, resolved_alg) = if self.config.jwks_url.contains("{kid}") {
            let url = self.config.jwks_url.replace("{kid}", kid);
            let key = self.fetch_single_key(&url, alg).await?;
            (key, alg)
        } else {
            self.fetch_jwks_key(&self.config.jwks_url, kid).await?
        };

        let mut cache = self.key_cache.write().await;
        cache.insert(
            kid.to_string(),
            CachedKey {
                decoding_key: key.clone(),
                algorithm: resolved_alg,
                fetched_at: Instant::now(),
            },
        );

        Ok((key, resolved_alg))
    }
}

// ── Algorithm resolution ────────────────────────────────────────────────

/// Map a JWK's `alg` field to a `jsonwebtoken::Algorithm`.
fn resolve_algorithm(jwk: &jsonwebtoken::jwk::Jwk) -> Result<Algorithm, OidcError> {
    match jwk.common.key_algorithm {
        Some(jsonwebtoken::jwk::KeyAlgorithm::ES256) => Ok(Algorithm::ES256),
        Some(jsonwebtoken::jwk::KeyAlgorithm::ES384) => Ok(Algorithm::ES384),
        Some(jsonwebtoken::jwk::KeyAlgorithm::RS256) => Ok(Algorithm::RS256),
        Some(jsonwebtoken::jwk::KeyAlgorithm::RS384) => Ok(Algorithm::RS384),
        Some(jsonwebtoken::jwk::KeyAlgorithm::RS512) => Ok(Algorithm::RS512),
        Some(jsonwebtoken::jwk::KeyAlgorithm::PS256) => Ok(Algorithm::PS256),
        Some(jsonwebtoken::jwk::KeyAlgorithm::PS384) => Ok(Algorithm::PS384),
        Some(jsonwebtoken::jwk::KeyAlgorithm::PS512) => Ok(Algorithm::PS512),
        Some(jsonwebtoken::jwk::KeyAlgorithm::EdDSA) => Ok(Algorithm::EdDSA),
        Some(other) => Err(OidcError::UnsupportedAlgorithm(format!("{other:?}"))),
        None => Err(OidcError::UnsupportedAlgorithm(
            "missing alg in JWK".to_string(),
        )),
    }
}

// ── Signature verification ──────────────────────────────────────────────

/// Verify the JWT signature using the **original** token text as the
/// signing input.
///
/// Why not just use `jsonwebtoken::decode()`?  Because `decode()` strips
/// base64 padding (`=`) from header and payload segments before building
/// the signing input.  AWS ALB signs over the *padded* segments, so
/// stripping padding changes the message and breaks verification.
///
/// We call `jsonwebtoken::crypto::verify()` directly with the original
/// `header.payload` bytes, then extract claims separately via
/// `decode()` with signature validation disabled (safe — we already
/// verified the signature above).
fn verify_signature(
    original_jwt: &str,
    key: &DecodingKey,
    alg: Algorithm,
) -> Result<(), OidcError> {
    let parts: Vec<&str> = original_jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(OidcError::InvalidSignature);
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let raw_sig = parts[2];

    // Decode signature bytes from base64url (tolerate padding).
    let sig_bytes = URL_SAFE_NO_PAD
        .decode(raw_sig.trim_end_matches('='))
        .map_err(|_| OidcError::InvalidSignature)?;

    // ECDSA signatures: handle DER encoding if present (some IdPs use
    // DER-encoded signatures instead of raw R||S).
    let sig_bytes = if matches!(alg, Algorithm::ES256 | Algorithm::ES384) {
        match try_der_to_raw(&sig_bytes, alg) {
            Some(raw) => raw,
            None => sig_bytes,
        }
    } else {
        sig_bytes
    };

    // Re-encode the (possibly DER→raw converted) signature to base64url
    // because jsonwebtoken::crypto::verify() expects a base64url string.
    let sig_b64 = URL_SAFE_NO_PAD.encode(&sig_bytes);

    // verify(signature_b64, message_bytes, key, alg)
    let valid = jsonwebtoken::crypto::verify(
        &sig_b64,
        signing_input.as_bytes(),
        key,
        alg,
    )
    .map_err(|_| OidcError::InvalidSignature)?;

    if valid {
        Ok(())
    } else {
        Err(OidcError::InvalidSignature)
    }
}

// ── Base64 normalization (for claim extraction only) ────────────────────

/// Strip base64 padding from a single segment.
///
/// Used only when building a normalized JWT for `jsonwebtoken::decode()`
/// claim extraction.  The `jsonwebtoken` crate uses `URL_SAFE_NO_PAD`
/// internally, so padded segments cause decode failures.
fn normalize_b64_segment(seg: &str) -> String {
    seg.trim_end_matches('=').to_string()
}

/// Rebuild the JWT with padding stripped from all three segments.
///
/// This is a no-op for RFC-compliant JWTs that already omit padding.
/// Only used for claim extraction after signature verification.
fn normalize_jwt_for_claims(jwt: &str) -> String {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return jwt.to_string();
    }
    format!(
        "{}.{}.{}",
        normalize_b64_segment(parts[0]),
        normalize_b64_segment(parts[1]),
        normalize_b64_segment(parts[2]),
    )
}

// ── DER → raw ECDSA signature conversion ────────────────────────────────

/// Try to convert a DER-encoded ECDSA signature to raw R||S format.
///
/// Returns `None` if the input doesn't look like valid DER, in which case
/// the caller should use the bytes as-is (already raw R||S).
fn try_der_to_raw(der: &[u8], alg: Algorithm) -> Option<Vec<u8>> {
    let component_len = match alg {
        Algorithm::ES256 => 32,
        Algorithm::ES384 => 48,
        _ => return None,
    };

    // DER SEQUENCE: 0x30 <len> <r_integer> <s_integer>
    if der.len() < 6 || der[0] != 0x30 {
        return None;
    }

    let mut pos = 2; // skip SEQUENCE tag + length

    // Parse R INTEGER
    if pos >= der.len() || der[pos] != 0x02 {
        return None;
    }
    pos += 1;
    let r_len = *der.get(pos)? as usize;
    pos += 1;
    let r_bytes = der.get(pos..pos + r_len)?;
    pos += r_len;

    // Parse S INTEGER
    if pos >= der.len() || der[pos] != 0x02 {
        return None;
    }
    pos += 1;
    let s_len = *der.get(pos)? as usize;
    pos += 1;
    let s_bytes = der.get(pos..pos + s_len)?;

    // Strip leading zero padding from DER INTEGER values and left-pad
    // to the expected component length.
    let r = strip_der_leading_zero(r_bytes);
    let s = strip_der_leading_zero(s_bytes);
    if r.len() > component_len || s.len() > component_len {
        return None;
    }

    let mut raw = vec![0u8; component_len * 2];
    raw[component_len - r.len()..component_len].copy_from_slice(r);
    raw[component_len * 2 - s.len()..].copy_from_slice(s);
    Some(raw)
}

/// Strip the leading zero byte that DER adds to unsigned INTEGERs when
/// the high bit is set (to distinguish from negative values).
fn strip_der_leading_zero(bytes: &[u8]) -> &[u8] {
    if bytes.len() > 1 && bytes[0] == 0x00 {
        &bytes[1..]
    } else {
        bytes
    }
}

// ── Full OIDC validation pipeline ───────────────────────────────────────

/// Validate an OIDC JWT: fetch key, verify signature, check claims.
///
/// Returns the `sub` (subject) claim on success.
async fn validate_oidc_jwt(oidc: &OidcState, jwt: &str) -> Result<String, OidcError> {
    // Normalize first — `decode_header()` uses URL_SAFE_NO_PAD internally
    // and chokes on the `=` padding that AWS ALB includes.
    let normalized = normalize_jwt_for_claims(jwt);

    // Decode the unverified header to get `kid` and `alg`.
    let header = jsonwebtoken::decode_header(&normalized)
        .map_err(|e| OidcError::InvalidClaims(format!("malformed header: {e}")))?;
    let kid = header.kid.ok_or(OidcError::MissingKid)?;
    let alg = header.alg;

    // Fetch (or retrieve from cache) the signing key.
    let (key, resolved_alg) = oidc.get_or_fetch_key(&kid, alg).await?;

    // Verify signature against the ORIGINAL JWT text (preserving any
    // padding). ALB signed over the padded segments, so we must use the
    // original token as the signing input.
    verify_signature(jwt, &key, resolved_alg)?;

    // Extract claims from the normalized (padding-stripped) JWT.
    // Safe because we already verified the signature above.
    let mut validation = Validation::new(resolved_alg);
    validation.insecure_disable_signature_validation();

    if let Some(ref iss) = oidc.config.issuer {
        validation.set_issuer(&[iss]);
    } else {
        validation.validate_exp = true;
        validation.set_issuer::<String>(&[]);
    }
    if let Some(ref aud) = oidc.config.audience {
        validation.set_audience(&[aud]);
    } else {
        validation.validate_aud = false;
    }

    let data = jsonwebtoken::decode::<serde_json::Value>(&normalized, &key, &validation)
        .map_err(|e| OidcError::InvalidClaims(format!("{e}")))?;

    let sub = data
        .claims
        .get("sub")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(sub)
}

// ── Bearer token helpers (unchanged from pre-OIDC) ──────────────────────

/// Whether query-string token auth is allowed for this request.
///
/// Only GET requests to streaming endpoints may use `?token=xxx`. This
/// minimizes token-in-URL exposure on state-changing routes, where the token
/// would leak via server logs, Referer headers, and browser history.
///
/// Allowed endpoints:
/// - SSE: `/api/chat/events`, `/api/logs/events` (EventSource can't set headers)
/// - WebSocket: `/api/chat/ws` (WS upgrade can't set custom headers)
///
/// If you add a new SSE or WebSocket endpoint, add its path here.
fn allows_query_token_auth(request: &Request) -> bool {
    if request.method() != Method::GET {
        return false;
    }

    matches!(
        request.uri().path(),
        "/api/chat/events" | "/api/logs/events" | "/api/chat/ws"
    )
}

/// Extract the `token` query parameter value, URL-decoded.
fn query_token(request: &Request) -> Option<String> {
    let query = request.uri().query()?;
    url::form_urlencoded::parse(query.as_bytes()).find_map(|(k, v)| {
        if k == "token" {
            Some(v.into_owned())
        } else {
            None
        }
    })
}

// ── Middleware ───────────────────────────────────────────────────────────

/// Auth middleware: bearer token → OIDC JWT → query param → 401.
///
/// SSE connections can't set headers from `EventSource`, so we also accept
/// `?token=xxx` as a query parameter, but only on SSE/WS endpoints.
pub async fn auth_middleware(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    // 1. Try Authorization: Bearer header (constant-time comparison).
    //    RFC 6750 §2.1: auth-scheme comparison is case-insensitive.
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && value.len() > 7
        && value[..7].eq_ignore_ascii_case("Bearer ")
        && bool::from(value.as_bytes()[7..].ct_eq(auth.token.as_bytes()))
    {
        return next.run(request).await;
    }

    // 2. Try OIDC JWT from configured header (if enabled).
    if let Some(ref oidc) = auth.oidc
        && let Some(jwt_header) = headers.get(oidc.header_name())
        && let Ok(jwt) = jwt_header.to_str()
    {
        match validate_oidc_jwt(oidc, jwt).await {
            Ok(sub) => {
                tracing::debug!(sub = %sub, "OIDC auth succeeded");
                return next.run(request).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "OIDC auth failed");
            }
        }
    }

    // 3. Fall back to query parameter (SSE/WS endpoints only, constant-time).
    if allows_query_token_auth(&request)
        && let Some(token) = query_token(&request)
        && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
    {
        return next.run(request).await;
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::credentials::{TEST_AUTH_SECRET_TOKEN, TEST_BEARER_TOKEN};

    #[test]
    fn test_auth_state_clone() {
        let state = AuthState {
            token: TEST_BEARER_TOKEN.to_string(),
            oidc: None,
        };
        let cloned = state.clone();
        assert_eq!(cloned.token, TEST_BEARER_TOKEN);
    }

    use axum::Router;
    use axum::body::Body;
    use axum::middleware;
    use axum::routing::{get, post};
    use tower::ServiceExt;

    async fn dummy_handler() -> &'static str {
        "ok"
    }

    /// Router with streaming endpoints (query auth allowed) and regular
    /// endpoints (query auth rejected).
    fn test_app(token: &str) -> Router {
        let state = AuthState {
            token: token.to_string(),
            oidc: None,
        };
        Router::new()
            .route("/api/chat/events", get(dummy_handler))
            .route("/api/logs/events", get(dummy_handler))
            .route("/api/chat/ws", get(dummy_handler))
            .route("/api/chat/history", get(dummy_handler))
            .route("/api/chat/send", post(dummy_handler))
            .layer(middleware::from_fn_with_state(state, auth_middleware))
    }

    #[tokio::test]
    async fn test_valid_bearer_token_passes() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", format!("Bearer {TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_invalid_bearer_token_rejected() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_query_token_allowed_for_chat_events() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri(format!("/api/chat/events?token={TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_query_token_allowed_for_logs_events() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri(format!("/api/logs/events?token={TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_query_token_allowed_for_ws_upgrade() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri(format!("/api/chat/ws?token={TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_query_token_url_encoded() {
        // Token with characters that get percent-encoded in URLs.
        let raw_token = "tok+en/with spaces";
        let app = test_app(raw_token);
        let req = Request::builder()
            .uri("/api/chat/events?token=tok%2Ben%2Fwith%20spaces")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_query_token_url_encoded_mismatch() {
        let app = test_app("real-token");
        // Encoded value decodes to "wrong-token", not "real-token".
        let req = Request::builder()
            .uri("/api/chat/events?token=wrong%2Dtoken")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_query_token_rejected_for_non_sse_get() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri(format!("/api/chat/history?token={TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_query_token_rejected_for_post() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/chat/send?token={TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_query_token_invalid_rejected() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri("/api/chat/events?token=wrong-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_no_auth_at_all_rejected() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri("/api/chat/events")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_bearer_header_works_for_post() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/chat/send")
            .header("Authorization", format!("Bearer {TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_bearer_prefix_case_insensitive() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", format!("bearer {TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_bearer_prefix_mixed_case() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", format!("BEARER {TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_empty_bearer_token_rejected() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", "Bearer ")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_token_with_whitespace_rejected() {
        let app = test_app(TEST_AUTH_SECRET_TOKEN);
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", format!("Bearer  {TEST_AUTH_SECRET_TOKEN}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ── OIDC unit tests ─────────────────────────────────────────────────

    #[test]
    fn test_normalize_jwt_noop_for_rfc_compliant() {
        // No padding → no change.
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.sig";
        assert_eq!(normalize_jwt_for_claims(jwt), jwt);
    }

    #[test]
    fn test_normalize_jwt_strips_padding() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9==.eyJzdWIiOiJ0ZXN0In0=.c2ln";
        let normalized = normalize_jwt_for_claims(jwt);
        assert!(!normalized.contains('='));
        assert!(normalized.starts_with("eyJhbGciOiJIUzI1NiJ9."));
    }

    #[test]
    fn test_normalize_b64_segment_no_padding() {
        assert_eq!(normalize_b64_segment("abc"), "abc");
    }

    #[test]
    fn test_normalize_b64_segment_with_padding() {
        assert_eq!(normalize_b64_segment("abc=="), "abc");
    }

    #[test]
    fn test_try_der_to_raw_non_der_passthrough() {
        // 64 bytes of raw R||S — not DER, should return None.
        let raw = vec![0x01; 64];
        assert!(try_der_to_raw(&raw, Algorithm::ES256).is_none());
    }

    #[test]
    fn test_try_der_to_raw_valid_der() {
        // Construct a minimal DER ECDSA signature for ES256.
        // SEQUENCE { INTEGER(r=1, 32 bytes), INTEGER(s=2, 32 bytes) }
        let r = vec![0x01; 32];
        let s = vec![0x02; 32];
        let mut der = vec![0x30, 68]; // SEQUENCE, length=68
        der.push(0x02);
        der.push(32);
        der.extend_from_slice(&r);
        der.push(0x02);
        der.push(32);
        der.extend_from_slice(&s);

        let raw = try_der_to_raw(&der, Algorithm::ES256).expect("should parse DER");
        assert_eq!(raw.len(), 64);
        assert_eq!(&raw[..32], &r[..]);
        assert_eq!(&raw[32..], &s[..]);
    }

    #[test]
    fn test_try_der_to_raw_with_leading_zero() {
        // DER adds a 0x00 prefix when the high bit of an INTEGER is set.
        let r = {
            let mut v = vec![0x00]; // leading zero
            v.extend_from_slice(&[0x80; 32]); // 32 bytes with high bit set
            v
        };
        let s = vec![0x01; 32];
        let mut der = vec![0x30, 69]; // SEQUENCE, length = 33+32+4 = 69
        der.push(0x02);
        der.push(33); // r_len = 33 (with leading zero)
        der.extend_from_slice(&r);
        der.push(0x02);
        der.push(32);
        der.extend_from_slice(&s);

        let raw = try_der_to_raw(&der, Algorithm::ES256).expect("should parse DER");
        assert_eq!(raw.len(), 64);
        // R should have the leading zero stripped.
        assert_eq!(raw[0], 0x80);
    }

    #[test]
    fn test_strip_der_leading_zero() {
        assert_eq!(strip_der_leading_zero(&[0x00, 0x80, 0x01]), &[0x80, 0x01]);
        assert_eq!(strip_der_leading_zero(&[0x80, 0x01]), &[0x80, 0x01]);
        assert_eq!(strip_der_leading_zero(&[0x00]), &[0x00]); // single zero stays
    }

    #[test]
    fn test_verify_signature_rejects_tampered_payload() {
        use jsonwebtoken::{EncodingKey, Header};

        // Use HS256 for a self-contained unit test (no external keys).
        let secret = b"test-secret-at-least-256-bits!!!";
        let header = Header::new(Algorithm::HS256);
        let claims = serde_json::json!({"sub": "alice", "exp": 9999999999u64});
        let token = jsonwebtoken::encode(
            &header,
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap();

        // Valid signature should pass.
        let key = DecodingKey::from_secret(secret);
        assert!(verify_signature(&token, &key, Algorithm::HS256).is_ok());

        // Tamper with the payload — signature should fail.
        let parts: Vec<&str> = token.split('.').collect();
        let tampered = format!("{}.{}.{}", parts[0], "dGFtcGVyZWQ", parts[2]);
        assert!(verify_signature(&tampered, &key, Algorithm::HS256).is_err());
    }
}
