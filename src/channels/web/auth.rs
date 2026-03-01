//! Bearer token authentication middleware for the web gateway.
//!
//! Supports multi-user mode: each token maps to a `UserIdentity` that carries
//! the user_id. The identity is inserted into request extensions so downstream
//! handlers can extract it via `AuthenticatedUser`.

use std::collections::HashMap;

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Identity resolved from a bearer token.
#[derive(Debug, Clone)]
pub struct UserIdentity {
    pub user_id: String,
    /// Additional user scopes this identity can read from.
    pub workspace_read_scopes: Vec<String>,
}

/// Multi-user auth state: maps tokens to user identities.
///
/// In single-user mode (the default), contains exactly one entry.
#[derive(Clone)]
pub struct MultiAuthState {
    tokens: HashMap<String, UserIdentity>,
}

impl MultiAuthState {
    /// Create a single-user auth state (backwards compatible).
    pub fn single(token: String, user_id: String) -> Self {
        let mut tokens = HashMap::new();
        tokens.insert(
            token,
            UserIdentity {
                user_id,
                workspace_read_scopes: Vec::new(),
            },
        );
        Self { tokens }
    }

    /// Create a multi-user auth state from a map of tokens to identities.
    pub fn multi(tokens: HashMap<String, UserIdentity>) -> Self {
        Self { tokens }
    }

    /// Authenticate a token, returning the associated identity if valid.
    ///
    /// Uses direct HashMap lookup for O(1) performance. This is not
    /// constant-time: an attacker observing response timing could
    /// distinguish "token not found" from "token found" (and potentially
    /// leak hash-bucket information). In practice, bearer tokens over
    /// TLS are high-entropy random strings where timing attacks are
    /// infeasible, and the prior O(N) iteration leaked more timing
    /// information by short-circuiting on the first match.
    pub fn authenticate(&self, candidate: &str) -> Option<&UserIdentity> {
        self.tokens.get(candidate)
    }

    /// Get the first token (for backwards-compatible printing at startup).
    pub fn first_token(&self) -> Option<&str> {
        self.tokens.keys().next().map(|s| s.as_str())
    }

    /// Get the first user identity (for single-user fallback).
    pub fn first_identity(&self) -> Option<&UserIdentity> {
        self.tokens.values().next()
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

/// Auth middleware that validates bearer token from header or query param.
///
/// SSE connections can't set headers from `EventSource`, so we also accept
/// `?token=xxx` as a query parameter.
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

    // Fall back to query parameter for SSE EventSource
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some(token) = pair.strip_prefix("token=")
                && let Some(identity) = auth.authenticate(token)
            {
                request.extensions_mut().insert(identity.clone());
                return next.run(request).await;
            }
        }
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

// Keep the old type as an alias for any external references during migration.
pub type AuthState = MultiAuthState;

#[cfg(test)]
mod tests {
    use super::*;

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

    // === QA Plan - Web gateway auth tests ===

    use axum::Router;
    use axum::body::Body;
    use axum::middleware;
    use axum::routing::get;
    use tower::ServiceExt;

    async fn dummy_handler() -> &'static str {
        "ok"
    }

    fn test_app(token: &str) -> Router {
        let state = MultiAuthState::single(token.to_string(), "test-user".to_string());
        Router::new()
            .route("/test", get(dummy_handler))
            .layer(middleware::from_fn_with_state(state, auth_middleware))
    }

    #[tokio::test]
    async fn test_valid_bearer_token_passes() {
        let app = test_app("secret-token");
        let req = Request::builder()
            .uri("/test")
            .header("Authorization", "Bearer secret-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_invalid_bearer_token_rejected() {
        let app = test_app("secret-token");
        let req = Request::builder()
            .uri("/test")
            .header("Authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_missing_auth_header_falls_through_to_query() {
        let app = test_app("secret-token");
        let req = Request::builder()
            .uri("/test?token=secret-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_query_param_invalid_token_rejected() {
        let app = test_app("secret-token");
        let req = Request::builder()
            .uri("/test?token=wrong-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_no_auth_at_all_rejected() {
        let app = test_app("secret-token");
        let req = Request::builder().uri("/test").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_bearer_prefix_case_insensitive() {
        // RFC 6750 Section 2.1: auth-scheme comparison must be case-insensitive.
        let app = test_app("secret-token");
        let req = Request::builder()
            .uri("/test")
            .header("Authorization", "bearer secret-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_bearer_prefix_mixed_case() {
        let app = test_app("secret-token");
        let req = Request::builder()
            .uri("/test")
            .header("Authorization", "BEARER secret-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_empty_bearer_token_rejected() {
        let app = test_app("secret-token");
        let req = Request::builder()
            .uri("/test")
            .header("Authorization", "Bearer ")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_token_with_whitespace_rejected() {
        // Extra space after "Bearer " means the token value starts with a space,
        // which should not match the expected token.
        let app = test_app("secret-token");
        let req = Request::builder()
            .uri("/test")
            .header("Authorization", "Bearer  secret-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
