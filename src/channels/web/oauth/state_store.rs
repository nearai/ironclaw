//! In-memory store for pending OAuth flows.
//!
//! Each OAuth login generates a CSRF state token and a PKCE code verifier.
//! These are stored here temporarily (5 min TTL) until the OAuth callback
//! completes the exchange. Entries are single-use (taken on callback) and
//! bounded to prevent memory exhaustion.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine;
use rand::RngCore;
use rand::rngs::OsRng;
use tokio::sync::RwLock;

const STATE_TTL: Duration = Duration::from_secs(300); // 5 minutes
const MAX_PENDING_STATES: usize = 1024;

/// A pending OAuth flow awaiting callback completion.
pub struct PendingOAuthFlow {
    /// Provider name (e.g. "google", "github").
    pub provider: String,
    /// PKCE code verifier (base64url-encoded, 43 chars).
    pub code_verifier: String,
    /// Optional URL to redirect to after login completes.
    pub redirect_after: Option<String>,
    created_at: Instant,
}

/// Thread-safe in-memory store for pending OAuth flows.
#[derive(Default)]
pub struct OAuthStateStore {
    states: RwLock<HashMap<String, PendingOAuthFlow>>,
}

impl OAuthStateStore {
    pub fn new() -> Self {
        Self {
            states: RwLock::new(HashMap::new()),
        }
    }

    /// Generate a PKCE code verifier (32 random bytes, base64url-encoded).
    pub fn generate_code_verifier() -> String {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Compute the S256 code challenge from a code verifier.
    pub fn code_challenge(verifier: &str) -> String {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(verifier.as_bytes());
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
    }

    /// Insert a new pending OAuth flow. Returns the CSRF state token.
    ///
    /// If at capacity, expired entries are evicted first. If still at
    /// capacity after eviction, the oldest entry is removed.
    pub async fn insert(&self, flow: PendingOAuthFlow) -> String {
        let mut states = self.states.write().await;

        // Evict expired entries if near capacity
        if states.len() >= MAX_PENDING_STATES {
            let now = Instant::now();
            states.retain(|_, f| now.duration_since(f.created_at) < STATE_TTL);
        }

        // If still at capacity, remove the oldest
        if states.len() >= MAX_PENDING_STATES
            && let Some(oldest_key) = states
                .iter()
                .min_by_key(|(_, f)| f.created_at)
                .map(|(k, _)| k.clone())
        {
            states.remove(&oldest_key);
        }

        let state_token = generate_state_token();
        states.insert(state_token.clone(), flow);
        state_token
    }

    /// Remove and return the flow for a given state token.
    ///
    /// Returns `None` if not found or expired. Single-use: the entry is
    /// removed regardless.
    pub async fn take(&self, state: &str) -> Option<PendingOAuthFlow> {
        let mut states = self.states.write().await;
        let flow = states.remove(state)?;
        if Instant::now().duration_since(flow.created_at) >= STATE_TTL {
            return None;
        }
        Some(flow)
    }

    /// Remove expired entries. Call periodically from a background task.
    pub async fn sweep_expired(&self) {
        let mut states = self.states.write().await;
        let now = Instant::now();
        states.retain(|_, f| now.duration_since(f.created_at) < STATE_TTL);
    }
}

/// Generate a 32-byte hex-encoded CSRF state token.
fn generate_state_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Validate a redirect URL to prevent open redirect attacks.
/// Only allows relative paths starting with `/` (and not `//` which browsers
/// treat as protocol-relative URLs).
fn sanitize_redirect(url: Option<String>) -> Option<String> {
    url.filter(|u| u.starts_with('/') && !u.starts_with("//"))
}

/// Create a `PendingOAuthFlow` with a fresh code verifier.
pub fn new_oauth_flow(provider: String, redirect_after: Option<String>) -> PendingOAuthFlow {
    PendingOAuthFlow {
        provider,
        code_verifier: OAuthStateStore::generate_code_verifier(),
        redirect_after: sanitize_redirect(redirect_after),
        created_at: Instant::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_insert_and_take() {
        let store = OAuthStateStore::new();
        let flow = new_oauth_flow("google".to_string(), None);
        let verifier = flow.code_verifier.clone();
        let state = store.insert(flow).await;

        let taken = store.take(&state).await;
        assert!(taken.is_some());
        let taken = taken.unwrap();
        assert_eq!(taken.provider, "google");
        assert_eq!(taken.code_verifier, verifier);
    }

    #[tokio::test]
    async fn test_take_removes_entry() {
        let store = OAuthStateStore::new();
        let flow = new_oauth_flow("github".to_string(), None);
        let state = store.insert(flow).await;

        let first = store.take(&state).await;
        assert!(first.is_some());

        let second = store.take(&state).await;
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn test_take_unknown_returns_none() {
        let store = OAuthStateStore::new();
        let result = store.take("nonexistent").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_code_challenge_is_deterministic() {
        let verifier = "test-verifier-string";
        let c1 = OAuthStateStore::code_challenge(verifier);
        let c2 = OAuthStateStore::code_challenge(verifier);
        assert_eq!(c1, c2);
        assert!(!c1.is_empty());
    }

    #[tokio::test]
    async fn test_sweep_expired() {
        let store = OAuthStateStore::new();

        // Insert a flow with an already-expired timestamp (by manipulating internals)
        {
            let mut states = store.states.write().await;
            states.insert(
                "expired-state".to_string(),
                PendingOAuthFlow {
                    provider: "google".to_string(),
                    code_verifier: "v".to_string(),
                    redirect_after: None,
                    created_at: Instant::now() - Duration::from_secs(600),
                },
            );
            states.insert(
                "fresh-state".to_string(),
                PendingOAuthFlow {
                    provider: "github".to_string(),
                    code_verifier: "v".to_string(),
                    redirect_after: None,
                    created_at: Instant::now(),
                },
            );
        }

        store.sweep_expired().await;

        let states = store.states.read().await;
        assert_eq!(states.len(), 1);
        assert!(states.contains_key("fresh-state"));
    }
}
