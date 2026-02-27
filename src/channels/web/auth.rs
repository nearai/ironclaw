//! Bearer token authentication middleware for the web gateway.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;

/// Shared auth state injected via axum middleware state.
#[derive(Clone)]
pub struct AuthState {
    pub token: String,
}

/// Auth middleware that validates bearer token from header or query param.
///
/// SSE connections can't set headers from `EventSource`, so we also accept
/// `?token=xxx` as a query parameter.
pub async fn auth_middleware(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    // Try Authorization header first (constant-time comparison).
    // RFC 6750 Section 2.1: auth-scheme comparison is case-insensitive.
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && value.len() > 7
        && value[..7].eq_ignore_ascii_case("Bearer ")
        && bool::from(value.as_bytes()[7..].ct_eq(auth.token.as_bytes()))
    {
        return next.run(request).await;
    }

    // Fall back to query parameter for SSE EventSource (constant-time comparison)
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some(token) = pair.strip_prefix("token=")
                && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
            {
                return next.run(request).await;
            }
        }
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_state_clone() {
        let state = AuthState {
            token: "test-token".to_string(),
        };
        let cloned = state.clone();
        assert_eq!(cloned.token, "test-token");
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
        let state = AuthState {
            token: token.to_string(),
        };
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
