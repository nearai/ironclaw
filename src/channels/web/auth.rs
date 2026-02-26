//! Bearer token authentication middleware for the web gateway.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;

/// Shared auth state injected via axum middleware state.
#[derive(Clone)]
pub struct AuthState {
    pub token: String,
}

/// Whether query-string token auth is allowed for this request.
///
/// Restricting query auth to SSE read endpoints minimizes token-in-URL exposure
/// on state-changing routes.
fn allows_query_token_auth(request: &Request) -> bool {
    if request.method() != Method::GET {
        return false;
    }

    matches!(
        request.uri().path(),
        "/api/chat/events" | "/api/logs/events"
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
/// `?token=xxx` as a query parameter.
pub async fn auth_middleware(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    // Try Authorization header first (constant-time comparison)
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && let Some(token) = value.strip_prefix("Bearer ")
        && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
    {
        return next.run(request).await;
    }

    // Fall back to query parameter for SSE EventSource (constant-time comparison).
    if allows_query_token_auth(&request)
        && let Some(token) = query_token(&request)
        && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
    {
        return next.run(request).await;
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router, middleware,
        routing::{get, post},
    };
    use tower::ServiceExt;

    fn test_router(token: &str) -> Router {
        Router::new()
            .route("/api/chat/events", get(|| async { "ok" }))
            .route("/api/logs/events", get(|| async { "ok" }))
            .route("/api/chat/history", get(|| async { "ok" }))
            .route("/api/chat/send", post(|| async { "ok" }))
            .route_layer(middleware::from_fn_with_state(
                AuthState {
                    token: token.to_string(),
                },
                auth_middleware,
            ))
    }

    #[test]
    fn test_auth_state_clone() {
        let state = AuthState {
            token: "test-token".to_string(),
        };
        let cloned = state.clone();
        assert_eq!(cloned.token, "test-token");
    }

    #[tokio::test]
    async fn query_token_allowed_for_chat_events_get() {
        let app = test_router("test-token");
        let req = axum::http::Request::builder()
            .uri("/api/chat/events?token=test-token")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn query_token_allowed_for_logs_events_get() {
        let app = test_router("test-token");
        let req = axum::http::Request::builder()
            .uri("/api/logs/events?token=test-token")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn query_token_rejected_for_non_sse_endpoint() {
        let app = test_router("test-token");
        let req = axum::http::Request::builder()
            .uri("/api/chat/history?token=test-token")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn query_token_rejected_for_post_endpoint() {
        let app = test_router("test-token");
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/api/chat/send?token=test-token")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn header_bearer_still_works_for_post() {
        let app = test_router("test-token");
        let req = axum::http::Request::builder()
            .method(Method::POST)
            .uri("/api/chat/send")
            .header("authorization", "Bearer test-token")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
