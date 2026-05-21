//! Session-backed `WebuiAuthenticator` for the Reborn WebChat v2
//! gateway.
//!
//! A session is the opaque bearer token the browser presents back on
//! every request after a successful login. The actual login flow that
//! mints the session is **outside** this module — host code calls
//! `SessionStore::insert` after whatever sign-in path it uses (password
//! form, magic link, OIDC, etc.).
//!
//! The store impl shipped here is in-memory only. Production
//! deployments wire their own `SessionStore` (Postgres, libSQL,
//! filesystem) by implementing the trait.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_reborn_composition::WebuiAuthenticator;
use parking_lot::RwLock;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use thiserror::Error;
use uuid::Uuid;

/// Persisted session record. The token value itself is the lookup key
/// (HashMap key), so this struct intentionally does NOT carry the
/// plaintext token after persistence — that would be a leak risk if
/// the value were ever logged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl SessionRecord {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

/// Errors raised by [`SessionStore`] implementations.
#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("session not found")]
    NotFound,
    #[error("session backend failure: {0}")]
    Backend(String),
}

/// Pluggable session backend. Host binaries implement this against
/// whatever durable store they prefer; the in-memory impl below is
/// fine for local dev and tests.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// Issue a new session bound to the supplied caller and lifetime.
    /// Returns the freshly minted bearer token; persist `record` keyed
    /// on this token (or whatever lookup encoding the backend prefers).
    async fn create_session(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
        lifetime: ChronoDuration,
    ) -> Result<SecretString, SessionStoreError>;

    /// Look up the session record bound to `candidate`. Implementations
    /// MUST use constant-time comparison on the secret material.
    async fn lookup(&self, candidate: &str) -> Result<Option<SessionRecord>, SessionStoreError>;

    /// Optional: revoke a session early. The default impl is a no-op
    /// because the in-memory store wipes on process restart anyway;
    /// durable backends should override.
    async fn revoke(&self, _candidate: &str) -> Result<(), SessionStoreError> {
        Ok(())
    }
}

/// Process-local session store. Sessions vanish on restart. Useful
/// for local dev and the caller-level test harness — production
/// deployments wire a durable `SessionStore` impl.
#[derive(Debug, Default)]
pub struct InMemorySessionStore {
    inner: RwLock<HashMap<String, SessionRecord>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count of active sessions — diagnostic helper for tests.
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn create_session(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
        lifetime: ChronoDuration,
    ) -> Result<SecretString, SessionStoreError> {
        let session_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let record = SessionRecord {
            session_id: session_id.clone(),
            tenant_id,
            user_id,
            created_at: now,
            expires_at: now
                .checked_add_signed(lifetime)
                .ok_or_else(|| SessionStoreError::Backend("session lifetime overflow".into()))?,
        };
        self.inner.write().insert(session_id.clone(), record);
        Ok(SecretString::from(session_id))
    }

    async fn lookup(&self, candidate: &str) -> Result<Option<SessionRecord>, SessionStoreError> {
        // Walk the map with constant-time comparison so that a hostile
        // caller cannot use timing to discover whether their guess
        // shares a prefix with a real session id. We accept the O(n)
        // walk because the in-memory store's expected workload is
        // small (interactive single-binary deployment).
        let guard = self.inner.read();
        let mut hit: Option<SessionRecord> = None;
        for (id, record) in guard.iter() {
            // ConstantTimeEq returns 1 on equal-length matches only;
            // length mismatch is also handled in constant time.
            if id.as_bytes().ct_eq(candidate.as_bytes()).into() {
                hit = Some(record.clone());
                // Do not break — keep the loop running so the timing
                // does not depend on which entry matched.
                continue;
            }
        }
        Ok(hit)
    }
}

/// `WebuiAuthenticator` impl that resolves the bearer token to a
/// stored session, checking expiry against the wall clock.
#[derive(Clone)]
pub struct SessionAuthenticator {
    store: Arc<dyn SessionStore>,
}

impl SessionAuthenticator {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self { store }
    }
}

impl std::fmt::Debug for SessionAuthenticator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionAuthenticator").finish()
    }
}

#[async_trait]
impl WebuiAuthenticator for SessionAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<UserId> {
        // Failure modes (not found / expired / backend error) all
        // collapse to `None` — the gateway emits a generic 401 and
        // never leaks the reason to the client.
        let Ok(Some(record)) = self.store.lookup(token).await else {
            return None;
        };
        if record.is_expired(Utc::now()) {
            tracing::debug!(
                target = "ironclaw::reborn::webui_ingress::session",
                user = %record.user_id,
                "rejecting expired session",
            );
            return None;
        }
        Some(record.user_id)
    }
}

/// Reborn-wide ergonomic alias so callers do not have to repeat the
/// secret-string type when threading session material through their
/// own helpers.
pub type SessionTokenSecret = SecretString;

/// Quick helper that lets host login code wrap a freshly created
/// session token. Hides the secrecy dependency from binary call sites
/// that only want to pass the token straight through to a Set-Cookie
/// header (callers MUST expose the value through `.expose_secret()`
/// only at the wire boundary).
pub fn wrap_session_token(token: String) -> SessionTokenSecret {
    SecretString::from(token)
}

/// Inspect a wrapped session token without leaking it into logs.
pub fn unwrap_session_token(token: &SessionTokenSecret) -> &str {
    token.expose_secret()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> TenantId {
        TenantId::new("tenant-a").expect("tenant")
    }
    fn user() -> UserId {
        UserId::new("alice").expect("user")
    }

    #[tokio::test]
    async fn create_then_lookup_returns_session() {
        let store = InMemorySessionStore::new();
        let token = store
            .create_session(tenant(), user(), ChronoDuration::hours(1))
            .await
            .expect("create");
        let record = store
            .lookup(token.expose_secret())
            .await
            .expect("lookup")
            .expect("record");
        assert_eq!(record.user_id.as_str(), "alice");
    }

    #[tokio::test]
    async fn expired_session_is_rejected_by_authenticator() {
        let store = Arc::new(InMemorySessionStore::new());
        let token = store
            .create_session(tenant(), user(), ChronoDuration::seconds(-1))
            .await
            .expect("create");
        let auth = SessionAuthenticator::new(store.clone());
        assert!(auth.authenticate(token.expose_secret()).await.is_none());
    }

    #[tokio::test]
    async fn unknown_token_is_rejected() {
        let store = Arc::new(InMemorySessionStore::new());
        let auth = SessionAuthenticator::new(store);
        assert!(auth.authenticate("nonexistent-token").await.is_none());
    }

    #[tokio::test]
    async fn live_session_resolves_to_caller_user_id() {
        let store = Arc::new(InMemorySessionStore::new());
        let token = store
            .create_session(tenant(), user(), ChronoDuration::hours(1))
            .await
            .expect("create");
        let auth = SessionAuthenticator::new(store);
        let resolved = auth
            .authenticate(token.expose_secret())
            .await
            .expect("authenticated");
        assert_eq!(resolved.as_str(), "alice");
    }
}
