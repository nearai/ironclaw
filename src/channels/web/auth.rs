//! Bearer token authentication middleware for the web gateway.
//!
//! Supports multi-user mode: each token maps to a `UserIdentity` that carries
//! the user_id. The identity is inserted into request extensions so downstream
//! handlers can extract it via `AuthenticatedUser`.

use std::collections::HashMap;

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, Method, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Identity resolved from a bearer token.
#[derive(Debug, Clone)]
pub struct UserIdentity {
    pub user_id: String,
    /// Additional user scopes this identity can read from.
    pub workspace_read_scopes: Vec<String>,
}

/// Hash a token with SHA-256 for constant-size, timing-safe storage.
fn hash_token(token: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize().into()
}

/// Multi-user auth state: maps token hashes to user identities.
///
/// Tokens are SHA-256 hashed on construction so they are never stored in
/// plaintext. Authentication compares fixed-size (32-byte) digests using
/// constant-time comparison, eliminating both length-oracle timing leaks
/// and accidental token exposure in memory dumps.
///
/// In single-user mode (the default), contains exactly one entry.
#[derive(Clone)]
pub struct MultiAuthState {
    /// Maps SHA-256(token) → identity. Tokens are never stored in cleartext.
    hashed_tokens: Vec<([u8; 32], UserIdentity)>,
    /// Original first token kept only for single-user startup printing.
    /// Not used for authentication.
    display_token: Option<String>,
}

impl MultiAuthState {
    /// Create a single-user auth state (backwards compatible).
    pub fn single(token: String, user_id: String) -> Self {
        let hash = hash_token(&token);
        Self {
            hashed_tokens: vec![(
                hash,
                UserIdentity {
                    user_id,
                    workspace_read_scopes: Vec::new(),
                },
            )],
            display_token: Some(token),
        }
    }

    /// Create a multi-user auth state from a map of tokens to identities.
    pub fn multi(tokens: HashMap<String, UserIdentity>) -> Self {
        let hashed_tokens: Vec<([u8; 32], UserIdentity)> = tokens
            .into_iter()
            .map(|(tok, identity)| (hash_token(&tok), identity))
            .collect();
        Self {
            hashed_tokens,
            display_token: None,
        }
    }

    /// Authenticate a token, returning the associated identity if valid.
    ///
    /// Uses SHA-256 hashing + constant-time comparison (`subtle::ConstantTimeEq`)
    /// to prevent timing side-channels. Both the candidate and stored tokens are
    /// hashed to 32-byte digests, eliminating length-oracle leaks. Iterates all
    /// entries regardless of match to avoid early-exit timing differences.
    /// O(n) in the number of configured users — negligible for typical
    /// deployments (< 10 users).
    pub fn authenticate(&self, candidate: &str) -> Option<&UserIdentity> {
        let candidate_hash = hash_token(candidate);
        let mut matched: Option<&UserIdentity> = None;
        for (stored_hash, identity) in &self.hashed_tokens {
            if bool::from(candidate_hash.ct_eq(stored_hash)) {
                matched = Some(identity);
            }
        }
        matched
    }

    /// Get the first token for backwards-compatible printing at startup.
    ///
    /// Only available in single-user mode; returns `None` in multi-user mode
    /// to avoid exposing tokens.
    pub fn first_token(&self) -> Option<&str> {
        self.display_token.as_deref()
    }

    /// Get the first user identity (for single-user fallback).
    pub fn first_identity(&self) -> Option<&UserIdentity> {
        self.hashed_tokens.first().map(|(_, id)| id)
    }
}

/// Axum extractor that provides the authenticated user identity.
///
/// Only available on routes behind `auth_middleware`. Extracts the
/// `UserIdentity` that the middleware inserted into request extensions.
pub struct AuthenticatedUser(pub UserIdentity);

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<UserIdentity>()
            .cloned()
            .map(AuthenticatedUser)
            .ok_or((StatusCode::UNAUTHORIZED, "Not authenticated"))
    }
}

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

/// Auth middleware that validates bearer token from header or query param.
///
/// SSE connections can't set headers from `EventSource`, so we also accept
/// `?token=xxx` as a query parameter, but only on SSE/WS endpoints.
///
/// On successful authentication, inserts the matching `UserIdentity` into
/// request extensions for downstream extraction via `AuthenticatedUser`.
pub async fn auth_middleware(
    State(auth): State<MultiAuthState>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    // Try Authorization header first.
    // RFC 6750 Section 2.1: auth-scheme comparison is case-insensitive.
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && value.len() > 7
        && value[..7].eq_ignore_ascii_case("Bearer ")
        && let Some(identity) = auth.authenticate(&value[7..])
    {
        request.extensions_mut().insert(identity.clone());
        return next.run(request).await;
    }

    // Fall back to query parameter, but only for SSE/WS endpoints.
    if allows_query_token_auth(&request)
        && let Some(token) = query_token(&request)
        && let Some(identity) = auth.authenticate(&token)
    {
        request.extensions_mut().insert(identity.clone());
        return next.run(request).await;
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::credentials::TEST_AUTH_SECRET_TOKEN;

    #[test]
    fn test_multi_auth_state_single() {
        let state = MultiAuthState::single("tok-123".to_string(), "alice".to_string());
        let identity = state.authenticate("tok-123");
        assert!(identity.is_some());
        assert_eq!(identity.unwrap().user_id, "alice");
    }

    #[test]
    fn test_multi_auth_state_reject_wrong_token() {
        let state = MultiAuthState::single("tok-123".to_string(), "alice".to_string());
        assert!(state.authenticate("wrong-token").is_none());
    }

    #[test]
    fn test_multi_auth_state_multi_users() {
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-alice".to_string(),
            UserIdentity {
                user_id: "alice".to_string(),
                workspace_read_scopes: Vec::new(),
            },
        );
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: "bob".to_string(),
                workspace_read_scopes: Vec::new(),
            },
        );
        let state = MultiAuthState::multi(tokens);

        let alice = state.authenticate("tok-alice").unwrap();
        assert_eq!(alice.user_id, "alice");

        let bob = state.authenticate("tok-bob").unwrap();
        assert_eq!(bob.user_id, "bob");

        assert!(state.authenticate("tok-charlie").is_none());
    }

    #[test]
    fn test_multi_auth_state_first_token() {
        let state = MultiAuthState::single("my-token".to_string(), "user1".to_string());
        assert_eq!(state.first_token(), Some("my-token"));
    }

    #[test]
    fn test_multi_auth_state_first_identity() {
        let state = MultiAuthState::single("my-token".to_string(), "user1".to_string());
        let identity = state.first_identity().unwrap();
        assert_eq!(identity.user_id, "user1");
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
        let state = MultiAuthState::single(token.to_string(), "test-user".to_string());
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

    // --- Multi-tenant auth integration tests ---

    /// Handler that extracts `AuthenticatedUser` and returns the resolved user_id.
    async fn identity_handler(AuthenticatedUser(identity): AuthenticatedUser) -> String {
        identity.user_id
    }

    /// Handler that extracts `AuthenticatedUser` and returns workspace_read_scopes as JSON.
    async fn scopes_handler(AuthenticatedUser(identity): AuthenticatedUser) -> String {
        serde_json::to_string(&identity.workspace_read_scopes).unwrap()
    }

    /// Build a multi-user router where each token maps to a distinct identity.
    fn multi_user_app(tokens: HashMap<String, UserIdentity>) -> Router {
        let state = MultiAuthState::multi(tokens);
        Router::new()
            .route("/api/chat/events", get(identity_handler))
            .route("/api/chat/send", post(identity_handler))
            .route("/api/scopes", get(scopes_handler))
            .layer(middleware::from_fn_with_state(state, auth_middleware))
    }

    fn two_user_tokens() -> HashMap<String, UserIdentity> {
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-alice".to_string(),
            UserIdentity {
                user_id: "alice".to_string(),
                workspace_read_scopes: vec!["shared".to_string()],
            },
        );
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: "bob".to_string(),
                workspace_read_scopes: vec!["shared".to_string(), "alice".to_string()],
            },
        );
        tokens
    }

    #[tokio::test]
    async fn test_multi_user_alice_token_resolves_to_alice() {
        let app = multi_user_app(two_user_tokens());
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(body, "alice");
    }

    #[tokio::test]
    async fn test_multi_user_bob_token_resolves_to_bob() {
        let app = multi_user_app(two_user_tokens());
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(body, "bob");
    }

    #[tokio::test]
    async fn test_multi_user_sequential_tokens_resolve_independently() {
        // Send both alice and bob tokens sequentially and verify each gets
        // the correct identity — guards against token map corruption.
        let tokens = two_user_tokens();

        let app1 = multi_user_app(tokens.clone());
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app1.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(body, "alice");

        let app2 = multi_user_app(tokens);
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(body, "bob");
    }

    #[tokio::test]
    async fn test_multi_user_unknown_token_rejected() {
        let app = multi_user_app(two_user_tokens());
        let req = Request::builder()
            .uri("/api/chat/events")
            .header("Authorization", "Bearer tok-charlie")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_multi_user_workspace_read_scopes_propagated() {
        let app = multi_user_app(two_user_tokens());

        // Alice has ["shared"]
        let req = Request::builder()
            .uri("/api/scopes")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let scopes: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert_eq!(scopes, vec!["shared"]);
    }

    #[tokio::test]
    async fn test_multi_user_bob_has_two_scopes() {
        let app = multi_user_app(two_user_tokens());

        // Bob has ["shared", "alice"]
        let req = Request::builder()
            .uri("/api/scopes")
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let scopes: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert_eq!(scopes, vec!["shared", "alice"]);
    }

    #[tokio::test]
    async fn test_multi_user_query_param_resolves_correct_identity() {
        let app = multi_user_app(two_user_tokens());
        let req = Request::builder()
            .uri("/api/chat/events?token=tok-bob")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(body, "bob");
    }

    #[tokio::test]
    async fn test_multi_user_post_with_bearer_resolves_identity() {
        let app = multi_user_app(two_user_tokens());
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/chat/send")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(body, "alice");
    }

    #[tokio::test]
    async fn test_multi_user_empty_scopes_for_single_user() {
        // Single-user mode creates identity with empty workspace_read_scopes.
        let state = MultiAuthState::single("tok-only".to_string(), "solo".to_string());
        let app = Router::new()
            .route("/api/scopes", get(scopes_handler))
            .layer(middleware::from_fn_with_state(state, auth_middleware));
        let req = Request::builder()
            .uri("/api/scopes")
            .header("Authorization", "Bearer tok-only")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let scopes: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert!(scopes.is_empty());
    }

    #[tokio::test]
    async fn test_prefix_and_extension_tokens_rejected() {
        // Verifies that prefix/suffix variants of valid tokens are rejected.
        // Note: the constant-time property is enforced structurally by use of
        // subtle::ConstantTimeEq and cannot be verified via outcome testing.
        let state = MultiAuthState::single("long-secret-token".to_string(), "user".to_string());
        assert!(state.authenticate("long-secret").is_none());
        assert!(state.authenticate("long-secret-token-extra").is_none());
    }
}
