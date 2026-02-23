//! Bearer token authentication middleware for the web gateway.
//!
//! Supports multi-user mode: each token maps to a `UserIdentity` that carries
//! the user_id, workspace read scopes, and memory layers. The identity is
//! inserted into request extensions so downstream handlers can extract it via
//! `AuthenticatedUser`.

use std::collections::HashMap;

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;

/// Identity resolved from a bearer token.
#[derive(Debug, Clone)]
pub struct UserIdentity {
    pub user_id: String,
    pub workspace_read_scopes: Vec<String>,
    pub memory_layers: Vec<crate::workspace::layer::MemoryLayer>,
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
                workspace_read_scopes: Vec::new(),
                memory_layers: crate::workspace::layer::MemoryLayer::default_for_user(&user_id),
                user_id,
            },
        );
        Self { tokens }
    }

    /// Create a single-user auth state with full identity fields.
    pub fn single_with_identity(token: String, identity: UserIdentity) -> Self {
        let mut tokens = HashMap::new();
        tokens.insert(token, identity);
        Self { tokens }
    }

    /// Create a multi-user auth state from a map of tokens to identities.
    pub fn multi(tokens: HashMap<String, UserIdentity>) -> Self {
        Self { tokens }
    }

    /// Authenticate a token, returning the associated identity if valid.
    ///
    /// Uses constant-time comparison to prevent timing attacks.
    pub fn authenticate(&self, candidate: &str) -> Option<&UserIdentity> {
        for (token, identity) in &self.tokens {
            if bool::from(candidate.as_bytes().ct_eq(token.as_bytes())) {
                return Some(identity);
            }
        }
        None
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
    // Try Authorization header first (constant-time comparison)
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && let Some(token) = value.strip_prefix("Bearer ")
        && let Some(identity) = auth.authenticate(token)
    {
        request.extensions_mut().insert(identity.clone());
        return next.run(request).await;
    }

    // Fall back to query parameter for SSE EventSource (constant-time comparison)
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
                workspace_read_scopes: vec![],
                memory_layers: vec![],
            },
        );
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: "bob".to_string(),
                workspace_read_scopes: vec!["shared".to_string()],
                memory_layers: vec![],
            },
        );
        let state = MultiAuthState::multi(tokens);

        let alice = state.authenticate("tok-alice").unwrap();
        assert_eq!(alice.user_id, "alice");

        let bob = state.authenticate("tok-bob").unwrap();
        assert_eq!(bob.user_id, "bob");
        assert_eq!(bob.workspace_read_scopes, vec!["shared"]);

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
}
