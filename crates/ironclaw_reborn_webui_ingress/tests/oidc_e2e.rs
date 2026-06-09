//! End-to-end OIDC authenticator tests. Generate a fresh RSA keypair
//! per test, mint signed JWTs with the private key, serve the public
//! key via a tiny axum-based JWKS endpoint on loopback, then drive
//! `OidcAuthenticator::authenticate` through the full
//! JWKS-fetch / kid-lookup / signature-verify / claim-check path.
//!
//! Bridges the gap the helper-level unit tests can't cover: a
//! regression in `decode_token` (e.g. accidentally widening the
//! algorithm allowlist, dropping iss/aud verification, breaking the
//! kid-miss force-refresh) would not show up in the helper tests but
//! does show up here.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use base64::Engine;
use ironclaw_reborn_composition::WebuiAuthenticator;
use ironclaw_reborn_webui_ingress::{OidcAuthenticator, OidcAuthenticatorConfig};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde_json::json;

const TEST_ISSUER: &str = "https://issuer.test";
const TEST_AUDIENCE: &str = "test-audience";
const TEST_KID: &str = "test-key-1";
const ROTATED_KID: &str = "test-key-2";

struct TestKey {
    private_pem: String,
    public: RsaPublicKey,
}

fn generate_test_key() -> TestKey {
    let mut rng = rand::thread_rng();
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("rsa gen");
    let pem = private
        .to_pkcs8_pem(LineEnding::LF)
        .expect("pkcs8 pem")
        .to_string();
    let public = RsaPublicKey::from(&private);
    TestKey {
        private_pem: pem,
        public,
    }
}

fn jwk_for(public: &RsaPublicKey, kid: &str) -> serde_json::Value {
    let n = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public.n().to_bytes_be());
    let e = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public.e().to_bytes_be());
    json!({
        "kty": "RSA",
        "alg": "RS256",
        "use": "sig",
        "kid": kid,
        "n": n,
        "e": e,
    })
}

#[derive(Clone)]
struct JwksState {
    keys: Arc<parking_lot::RwLock<Vec<serde_json::Value>>>,
    fetch_count: Arc<AtomicUsize>,
    fail_until: Arc<parking_lot::RwLock<Option<tokio::time::Instant>>>,
}

impl JwksState {
    fn new(keys: Vec<serde_json::Value>) -> Self {
        Self {
            keys: Arc::new(parking_lot::RwLock::new(keys)),
            fetch_count: Arc::new(AtomicUsize::new(0)),
            fail_until: Arc::new(parking_lot::RwLock::new(None)),
        }
    }

    fn replace_keys(&self, keys: Vec<serde_json::Value>) {
        *self.keys.write() = keys;
    }

    fn fetch_count(&self) -> usize {
        self.fetch_count.load(Ordering::SeqCst)
    }

    fn fail_for(&self, duration: Duration) {
        *self.fail_until.write() = Some(tokio::time::Instant::now() + duration);
    }
}

async fn jwks_handler(State(state): State<JwksState>) -> axum::response::Response {
    state.fetch_count.fetch_add(1, Ordering::SeqCst);
    if let Some(until) = *state.fail_until.read()
        && tokio::time::Instant::now() < until
    {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "test-induced failure",
        )
            .into_response();
    }
    Json(json!({ "keys": &*state.keys.read() })).into_response()
}

async fn spawn_jwks_server(state: JwksState) -> (String, tokio::task::JoinHandle<()>) {
    let app = axum::Router::new()
        .route("/jwks", get(jwks_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/jwks"), handle)
}

fn sign_token(
    private_pem: &str,
    kid: &str,
    issuer: &str,
    audience: &str,
    expires_in_secs: i64,
) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": issuer,
        "sub": "alice",
        "aud": audience,
        "exp": now + expires_in_secs,
        "iat": now,
    });
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("encoding key"),
    )
    .expect("sign jwt")
}

fn build_authenticator(jwks_url: String) -> OidcAuthenticator {
    let mut config = OidcAuthenticatorConfig::new(TEST_ISSUER, TEST_AUDIENCE, jwks_url);
    config.jwks_cache_ttl = Some(Duration::from_secs(300));
    config.http_timeout = Some(Duration::from_secs(5));
    OidcAuthenticator::new(config, OidcAuthenticator::sub_claim_mapper()).expect("authenticator")
}

#[tokio::test]
async fn oidc_authenticator_accepts_valid_jwks_signed_token_and_rejects_bad_claims() {
    let key = generate_test_key();
    let state = JwksState::new(vec![jwk_for(&key.public, TEST_KID)]);
    let (jwks_url, server) = spawn_jwks_server(state.clone()).await;
    let auth = build_authenticator(jwks_url);

    // (1) Valid token → accepted, sub maps to UserId.
    let valid = sign_token(&key.private_pem, TEST_KID, TEST_ISSUER, TEST_AUDIENCE, 600);
    let user = auth
        .authenticate(&valid)
        .await
        .expect("valid JWKS-signed token must be accepted");
    assert_eq!(user.as_str(), "alice");

    // (2) Wrong issuer → rejected.
    let wrong_iss = sign_token(
        &key.private_pem,
        TEST_KID,
        "https://attacker.test",
        TEST_AUDIENCE,
        600,
    );
    assert!(
        auth.authenticate(&wrong_iss).await.is_none(),
        "JWT with wrong iss must be rejected",
    );

    // (3) Wrong audience → rejected.
    let wrong_aud = sign_token(
        &key.private_pem,
        TEST_KID,
        TEST_ISSUER,
        "wrong-audience",
        600,
    );
    assert!(
        auth.authenticate(&wrong_aud).await.is_none(),
        "JWT with wrong aud must be rejected",
    );

    // (4) Expired → rejected.
    let expired = sign_token(&key.private_pem, TEST_KID, TEST_ISSUER, TEST_AUDIENCE, -60);
    assert!(
        auth.authenticate(&expired).await.is_none(),
        "expired JWT must be rejected",
    );

    // (5) Garbage token → rejected (not a panic).
    assert!(auth.authenticate("not.a.jwt").await.is_none());

    server.abort();
}

#[tokio::test]
async fn oidc_authenticator_refetches_jwks_on_kid_miss_during_rotation() {
    // Simulate issuer key rotation: cache holds key-1, an incoming JWT
    // is signed with key-2. The authenticator must force-refresh JWKS
    // and accept the token, not 401 until the cache TTL expires.
    let key_one = generate_test_key();
    let key_two = generate_test_key();
    let state = JwksState::new(vec![jwk_for(&key_one.public, TEST_KID)]);
    let (jwks_url, server) = spawn_jwks_server(state.clone()).await;
    let auth = build_authenticator(jwks_url);

    // Warm the cache with key-1.
    let token_one = sign_token(
        &key_one.private_pem,
        TEST_KID,
        TEST_ISSUER,
        TEST_AUDIENCE,
        600,
    );
    auth.authenticate(&token_one)
        .await
        .expect("warm-cache token accepted");
    let fetch_after_warm = state.fetch_count();

    // Issuer rotates: cache replaced with key-2 only.
    state.replace_keys(vec![jwk_for(&key_two.public, ROTATED_KID)]);

    // A token signed with the rotated key must be accepted via the
    // kid-miss force-refresh path.
    let token_two = sign_token(
        &key_two.private_pem,
        ROTATED_KID,
        TEST_ISSUER,
        TEST_AUDIENCE,
        600,
    );
    let user = auth
        .authenticate(&token_two)
        .await
        .expect("rotated-key token must trigger force-refresh + accept");
    assert_eq!(user.as_str(), "alice");
    // The force-refresh produced exactly one additional JWKS fetch.
    assert_eq!(
        state.fetch_count(),
        fetch_after_warm + 1,
        "kid-miss must trigger exactly one extra JWKS fetch",
    );

    server.abort();
}

#[tokio::test]
async fn oidc_jwks_refresh_is_single_flight_under_concurrent_authenticate() {
    // 10 concurrent auth attempts against an empty cache must result
    // in exactly ONE JWKS fetch — the single-flight Mutex in jwks()
    // serializes the network path while still letting all 10 callers
    // complete using the freshly populated cache.
    let key = generate_test_key();
    let state = JwksState::new(vec![jwk_for(&key.public, TEST_KID)]);
    let (jwks_url, server) = spawn_jwks_server(state.clone()).await;
    let auth = Arc::new(build_authenticator(jwks_url));

    let token = Arc::new(sign_token(
        &key.private_pem,
        TEST_KID,
        TEST_ISSUER,
        TEST_AUDIENCE,
        600,
    ));
    let mut handles = Vec::new();
    for _ in 0..10 {
        let auth = auth.clone();
        let token = token.clone();
        handles.push(tokio::spawn(async move { auth.authenticate(&token).await }));
    }
    for handle in handles {
        let user = handle.await.expect("join").expect("auth accepted");
        assert_eq!(user.as_str(), "alice");
    }
    assert_eq!(
        state.fetch_count(),
        1,
        "single-flight must coalesce 10 concurrent first-time auths into 1 JWKS fetch",
    );

    server.abort();
}

#[tokio::test]
async fn oidc_jwks_failure_backoff_avoids_convoy_against_slow_upstream() {
    // After the first JWKS fetch fails, subsequent fetches within
    // JWKS_FAILURE_BACKOFF must NOT block on the network — they
    // either return stale keys (if any) or fail fast. Pin the
    // convoy-break behavior so a slow / unavailable JWKS endpoint
    // cannot stall every authenticated request behind another
    // timeout-length fetch.
    let key = generate_test_key();
    let state = JwksState::new(vec![jwk_for(&key.public, TEST_KID)]);
    let (jwks_url, server) = spawn_jwks_server(state.clone()).await;

    // Use a short cache TTL so we re-enter the network path quickly.
    let mut config = OidcAuthenticatorConfig::new(TEST_ISSUER, TEST_AUDIENCE, jwks_url);
    config.jwks_cache_ttl = Some(Duration::from_millis(50));
    config.http_timeout = Some(Duration::from_secs(5));
    let auth = Arc::new(
        OidcAuthenticator::new(config, OidcAuthenticator::sub_claim_mapper())
            .expect("authenticator"),
    );

    // Warm the cache.
    let token = sign_token(&key.private_pem, TEST_KID, TEST_ISSUER, TEST_AUDIENCE, 600);
    auth.authenticate(&token)
        .await
        .expect("warm-cache token accepted");

    // Expire the cache + start failing JWKS.
    tokio::time::sleep(Duration::from_millis(80)).await;
    state.fail_for(Duration::from_secs(60));
    let fetch_before_burst = state.fetch_count();

    // Burst 10 concurrent calls. The first will fetch, see the 500,
    // record last_failure_at, and serve stale keys. The remaining 9
    // should NOT each spawn their own fetch — they should see the
    // backoff window and reuse the stale cache.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let auth = auth.clone();
        let token = token.clone();
        handles.push(tokio::spawn(async move { auth.authenticate(&token).await }));
    }
    for handle in handles {
        let result = handle.await.expect("join");
        // Each call should still authenticate — the stale cache is
        // valid for the test JWT.
        assert!(
            result.is_some(),
            "stale-while-revalidate must keep auth working during backoff",
        );
    }
    let extra_fetches = state.fetch_count() - fetch_before_burst;
    assert!(
        extra_fetches <= 2,
        "backoff window must coalesce concurrent failures (saw {extra_fetches} extra fetches; \
         expected ≤2 — one that triggered the backoff, plus an at-most-one race)",
    );

    server.abort();
}

// Regression for the unknown-kid DoS review (Medium): every
// syntactically valid JWT with an unknown `kid` triggers the
// kid-miss force-refresh path inside `decode_token`. Without a
// minimum-interval backoff on that path, an unauthenticated caller
// can stream forged JWTs with rotating `kid` values and force one
// upstream JWKS fetch per request — DoSing both this gateway and
// the issuer before bearer auth ever succeeds (and before the
// per-caller rate limiter has a caller key to attribute against).
// The backoff bounds upstream fetches to at most one per
// `JWKS_FORCED_REFRESH_MIN_INTERVAL` regardless of how many unknown
// kids the attacker spins through.
#[tokio::test]
async fn oidc_unknown_kid_storm_does_not_force_unbounded_jwks_refresh() {
    let key = generate_test_key();
    let state = JwksState::new(vec![jwk_for(&key.public, TEST_KID)]);
    let (jwks_url, server) = spawn_jwks_server(state.clone()).await;
    let auth = build_authenticator(jwks_url);

    // Warm the cache so the first request doesn't count against the
    // unknown-kid budget.
    let known = sign_token(&key.private_pem, TEST_KID, TEST_ISSUER, TEST_AUDIENCE, 600);
    auth.authenticate(&known)
        .await
        .expect("warm-cache token accepted");
    let fetch_after_warm = state.fetch_count();

    // Stream 25 forged JWTs each claiming a fresh unknown `kid`.
    // Each one would (without backoff) trigger one extra upstream
    // fetch. With backoff, at most one extra fetch is permitted.
    // We sign with a DIFFERENT key so the lookup fails even after
    // the first forced refresh — the upstream JWKS only ever
    // contains the rotated key, not the attacker's.
    let attacker_key = generate_test_key();
    for n in 0..25 {
        let kid = format!("attacker-kid-{n}");
        let bad_token = sign_token(
            &attacker_key.private_pem,
            &kid,
            TEST_ISSUER,
            TEST_AUDIENCE,
            600,
        );
        // Each token must be REJECTED — security property preserved.
        assert!(
            auth.authenticate(&bad_token).await.is_none(),
            "JWT with unknown kid `{kid}` must be rejected",
        );
    }
    let extra_fetches = state.fetch_count() - fetch_after_warm;
    assert!(
        extra_fetches <= 1,
        "unknown-kid storm must NOT force >1 upstream JWKS fetch within the backoff window; \
         saw {extra_fetches} extra fetches",
    );

    server.abort();
}

/// Sign an HS256 (symmetric) token with an attacker-chosen secret. The
/// `kid` matches the JWKS RSA key so a naive verifier could be tricked
/// into the classic alg-confusion attack (verify an HS256 MAC using the
/// RSA public key as the secret) — the RS/ES-only allowlist must reject
/// it before that can happen.
fn sign_hs256_token(kid: &str, issuer: &str, audience: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": issuer,
        "sub": "alice",
        "aud": audience,
        "exp": now + 600,
        "iat": now,
    });
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(kid.to_string());
    encode(
        &header,
        &claims,
        &EncodingKey::from_secret(b"attacker-chosen-secret"),
    )
    .expect("sign hs256")
}

/// Sign an RS256 token with a `nbf` (not-before) claim `nbf_offset_secs`
/// from now (negative = in the past). All other claims are valid.
fn sign_token_with_nbf(
    private_pem: &str,
    kid: &str,
    issuer: &str,
    audience: &str,
    nbf_offset_secs: i64,
) -> String {
    let now = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": issuer,
        "sub": "alice",
        "aud": audience,
        "exp": now + 600,
        "iat": now,
        "nbf": now + nbf_offset_secs,
    });
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("encoding key"),
    )
    .expect("sign jwt")
}

#[tokio::test]
async fn oidc_authenticator_rejects_hs256_tokens() {
    // The parity doc (01-auth.md row 5) claims an RS/ES-only algorithm
    // allowlist that rejects HS256. `oidc_e2e.rs` otherwise only signs
    // RS256, so lock the symmetric-algorithm rejection end-to-end: an
    // HS256 token (valid iss/aud/exp, kid matching the JWKS key) must
    // not authenticate.
    let key = generate_test_key();
    let state = JwksState::new(vec![jwk_for(&key.public, TEST_KID)]);
    let (jwks_url, server) = spawn_jwks_server(state).await;
    let auth = build_authenticator(jwks_url);

    let hs256 = sign_hs256_token(TEST_KID, TEST_ISSUER, TEST_AUDIENCE);
    assert!(
        auth.authenticate(&hs256).await.is_none(),
        "HS256 token must be rejected by the RS/ES-only allowlist (alg-confusion defense)",
    );

    server.abort();
}

#[tokio::test]
async fn oidc_authenticator_rejects_future_nbf() {
    // The parity doc (row 5) lists `nbf` among the validated claims, but
    // `oidc.rs` only has a helper-level `in_future` unit test. Lock the
    // claim end-to-end through `authenticate()`: a token whose `nbf` is
    // in the future is rejected, while an otherwise-identical token with
    // a past `nbf` authenticates — isolating `nbf` as the cause.
    let key = generate_test_key();
    let state = JwksState::new(vec![jwk_for(&key.public, TEST_KID)]);
    let (jwks_url, server) = spawn_jwks_server(state).await;
    let auth = build_authenticator(jwks_url);

    let future_nbf =
        sign_token_with_nbf(&key.private_pem, TEST_KID, TEST_ISSUER, TEST_AUDIENCE, 3600);
    assert!(
        auth.authenticate(&future_nbf).await.is_none(),
        "a token whose nbf is in the future must be rejected",
    );

    let past_nbf = sign_token_with_nbf(
        &key.private_pem,
        TEST_KID,
        TEST_ISSUER,
        TEST_AUDIENCE,
        -3600,
    );
    let user = auth
        .authenticate(&past_nbf)
        .await
        .expect("a token with a past nbf must authenticate (isolates nbf as the cause)");
    assert_eq!(user.as_str(), "alice");

    server.abort();
}
