//! Stateless, HMAC-signed session login wiring — the host-owned,
//! production-suitable counterpart to the dev-only
//! [`InMemorySessionStore`](crate::session::InMemorySessionStore).
//!
//! [`build_signed_session_login`] assembles the pieces the
//! `ironclaw-reborn serve` binary needs to mount the OAuth login
//! surface, so the CLI only supplies env/boot config (provider client
//! ids/secrets, base URL, operator secret) plus a host-owned
//! [`UserDirectory`] and calls the builder — it does not own the
//! auth/session model. That keeps the rule from this crate's guardrails
//! intact: `WebuiAuthenticator` / `SessionStore` implementations live
//! here, not in the command crate.
//!
//! - [`SignedTokenSessionStore`] — a `SessionStore` whose bearer token
//!   carries the tenant/user/expiry, HMAC-SHA256-signed with a key
//!   derived from the operator secret. Validation needs no persistence,
//!   so tokens survive a restart as long as the operator secret is
//!   stable. Revocation IS honored within a process via an in-memory
//!   denylist, so `POST /auth/logout` truly invalidates the presented
//!   bearer rather than returning `204` while the token stays live. The
//!   denylist is process-local and clears on restart, after which a
//!   not-yet-expired revoked token would validate again; a deployment
//!   needing durable revocation supplies a DB-backed `SessionStore`.
//! - The `user_directory` (host-supplied via `SignedSessionLoginConfig`)
//!   maps each authenticated OAuth profile to a `UserId`. A multi-user
//!   host injects a DB-backed directory (a distinct user per identity);
//!   a single-operator host injects one that always resolves to the
//!   operator. Each session bearer then carries that per-user identity.
//! - [`CompositeAuthenticator`] — accepts EITHER a minted session token
//!   OR the host's existing env-bearer operator token.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_reborn_composition::{WebuiAuthentication, WebuiAuthenticator};
use parking_lot::RwLock;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::{
    OAuthProvider, OAuthRouterConfig, PublicRouteMount, UserDirectory, webui_v2_auth_router,
};
use crate::session::{
    SessionAuthenticator, SessionId, SessionRecord, SessionStore, SessionStoreError,
};

type HmacSha256 = Hmac<Sha256>;
const SESSION_EPOCH_MAX_CHARS: usize = 128;

/// Deployment epoch carried by signed WebUI session tokens.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct SessionEpoch(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionEpochError {
    #[error("session epoch must not be empty")]
    Empty,
    #[error("session epoch exceeds {max} chars")]
    TooLong { max: usize },
    #[error("session epoch must not contain control characters")]
    ControlCharacter,
}

impl SessionEpoch {
    pub fn new(raw: impl Into<String>) -> Result<Self, SessionEpochError> {
        let value = raw.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    fn validate(value: &str) -> Result<(), SessionEpochError> {
        if value.trim().is_empty() {
            return Err(SessionEpochError::Empty);
        }
        if value.chars().count() > SESSION_EPOCH_MAX_CHARS {
            return Err(SessionEpochError::TooLong {
                max: SESSION_EPOCH_MAX_CHARS,
            });
        }
        if value.chars().any(char::is_control) {
            return Err(SessionEpochError::ControlCharacter);
        }
        Ok(())
    }
}

impl TryFrom<String> for SessionEpoch {
    type Error = SessionEpochError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl AsRef<str> for SessionEpoch {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for SessionEpoch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<SessionEpoch> for String {
    fn from(value: SessionEpoch) -> Self {
        value.into_inner()
    }
}

/// Host-supplied input to [`build_signed_session_login`].
pub struct SignedSessionLoginConfig {
    /// Host-trusted installation tenant; never browser-influenced.
    pub tenant_id: TenantId,
    /// Host-supplied directory that maps each authenticated OAuth
    /// profile to a `UserId`. Multi-user deployments inject a DB-backed
    /// directory (distinct user per identity); a single-operator host
    /// can inject one that always resolves to the operator.
    pub user_directory: Arc<dyn UserDirectory>,
    /// Operator secret the session-token HMAC key is derived from. The
    /// same value typically backs the env-bearer authenticator.
    pub operator_secret: SecretString,
    /// Optional deployment epoch included in newly minted session tokens and
    /// required during lookup when configured. Changing this value forces all
    /// browsers through SSO again without rotating the broader operator secret.
    pub session_epoch: Option<SessionEpoch>,
    /// Host-supplied validator that re-checks whether a signed token's user
    /// still has active access. SSO creation still goes through
    /// `user_directory`; this guard invalidates stale signed bearers.
    pub session_user_access_validator: Arc<dyn SessionUserAccessValidator>,
    /// Public base URL used to build provider callback URLs.
    pub base_url: String,
    /// Configured OAuth providers. An empty list disables the login
    /// surface — [`build_signed_session_login`] returns `None`.
    pub providers: Vec<Arc<dyn OAuthProvider>>,
    /// The host's existing env-bearer authenticator, composed alongside
    /// the session authenticator so scripted `Authorization: Bearer`
    /// workflows keep working next to browser login.
    pub env_authenticator: Arc<dyn WebuiAuthenticator>,
}

/// The assembled SSO login surface: the public route mount to merge
/// into `webui_v2_app`, plus the authenticator the protected v2 routes
/// must use so a minted session bearer authenticates.
pub struct SignedSessionLoginWiring {
    pub mount: PublicRouteMount,
    pub authenticator: Arc<dyn WebuiAuthenticator>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionUserAccessError {
    #[error("session user access backend failure: {0}")]
    Backend(String),
}

#[async_trait]
pub trait SessionUserAccessValidator: Send + Sync + 'static {
    async fn has_session_access(
        &self,
        tenant_id: &TenantId,
        user_id: &UserId,
    ) -> Result<bool, SessionUserAccessError>;
}

/// Assemble the signed-token login surface from host config. Returns
/// `None` when no provider is configured, in which case the caller
/// keeps its plain env-bearer authenticator and mounts no public login
/// routes.
pub fn build_signed_session_login(
    config: SignedSessionLoginConfig,
) -> Option<SignedSessionLoginWiring> {
    if config.providers.is_empty() {
        return None;
    }

    let session_store: Arc<dyn SessionStore> =
        Arc::new(SignedTokenSessionStore::from_operator_secret(
            &config.operator_secret,
            &config.tenant_id,
            config.session_epoch,
            config.session_user_access_validator,
        ));
    let session_authenticator: Arc<dyn WebuiAuthenticator> =
        Arc::new(SessionAuthenticator::new(session_store.clone()));

    let router_config = OAuthRouterConfig::new(
        config.tenant_id,
        session_store,
        config.user_directory,
        config.providers,
        config.base_url,
    );
    let mount = webui_v2_auth_router(router_config);
    let authenticator: Arc<dyn WebuiAuthenticator> = Arc::new(CompositeAuthenticator::new(
        session_authenticator,
        config.env_authenticator,
    ));

    Some(SignedSessionLoginWiring {
        mount,
        authenticator,
    })
}

/// Hard cap on the revocation denylist, mirroring the bounded discipline
/// of the sibling `PendingFlowStore` / `SessionTicketStore`. Without it the
/// map would grow with logout-rate × session-lifetime (up to 30 days),
/// unbounded. At the cap, expired entries are swept and — if still full of
/// live entries — the one closest to expiry is dropped (it leaves the
/// denylist slightly early but its token still expires on schedule via
/// `exp`).
const MAX_REVOKED_ENTRIES: usize = 4096;

/// Stateless `SessionStore` whose "record" is the cryptographic
/// signature itself (HMAC-SHA256 over the base64url payload), plus a
/// process-local denylist that makes `revoke` (logout) effective.
struct SignedTokenSessionStore {
    key: Vec<u8>,
    /// Host tenant this store is bound to. The signing key is derived from
    /// it, and `lookup` re-checks it as defense in depth.
    tenant_id: TenantId,
    /// Optional deployment epoch. When configured, lookup only accepts tokens
    /// minted with this exact epoch.
    session_epoch: Option<SessionEpoch>,
    access_validator: Arc<dyn SessionUserAccessValidator>,
    /// Revoked session ids → their expiry (unix seconds). Bounded by
    /// [`MAX_REVOKED_ENTRIES`]; the common-case logout is an O(1) insert,
    /// with an expired-entry sweep only when the map reaches the cap (so
    /// the hot per-request `lookup` read-lock is not blocked by an O(n)
    /// scan under the write lock on every logout).
    revoked: RwLock<HashMap<String, i64>>,
}

impl SignedTokenSessionStore {
    /// Derive the HMAC key from the operator secret AND the host tenant,
    /// domain-separated so the session-signing key never collides with
    /// another use of the same secret. Binding the tenant into the key is
    /// the primary cross-tenant control: two `serve` instances that share
    /// one operator secret but serve different tenants derive different
    /// keys, so neither can validate the other's session tokens — a token
    /// minted for one tenant fails the HMAC check on the other.
    fn from_operator_secret(
        secret: &SecretString,
        tenant_id: &TenantId,
        session_epoch: Option<SessionEpoch>,
        access_validator: Arc<dyn SessionUserAccessValidator>,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"ironclaw-reborn-webui-session-v1::");
        // Length-prefix the tenant so its bytes can never be confused with
        // the secret bytes that follow (a tenant of `a` + secret `bc` must
        // not key-collide with tenant `ab` + secret `c`).
        let tenant_bytes = tenant_id.as_str().as_bytes();
        hasher.update((tenant_bytes.len() as u64).to_le_bytes());
        hasher.update(tenant_bytes);
        hasher.update(b"::");
        hasher.update(secret.expose_secret().as_bytes());
        Self {
            key: hasher.finalize().to_vec(),
            tenant_id: tenant_id.clone(),
            session_epoch,
            access_validator,
            revoked: RwLock::new(HashMap::new()),
        }
    }

    /// Fresh keyed MAC. `new_from_slice` is infallible for HMAC — it
    /// accepts a key of any length — so the `expect` can never fire.
    fn mac(&self) -> HmacSha256 {
        HmacSha256::new_from_slice(&self.key).expect("HMAC-SHA256 accepts a key of any length") // safety: HMAC new_from_slice is infallible — it accepts a key of any length
    }

    fn sign(&self, payload_b64: &str) -> String {
        let mut mac = self.mac();
        mac.update(payload_b64.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    /// Split, constant-time-verify, and decode a candidate bearer.
    /// Returns the payload only when the signature is valid; any
    /// structural or signature failure yields `None` (an auth miss,
    /// not an error).
    fn verify(&self, candidate: &str) -> Option<TokenPayload> {
        let (payload_b64, signature_b64) = candidate.split_once('.')?;
        let signature = URL_SAFE_NO_PAD.decode(signature_b64).ok()?;
        let mut mac = self.mac();
        mac.update(payload_b64.as_bytes());
        mac.verify_slice(&signature).ok()?;
        let payload_json = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
        serde_json::from_slice::<TokenPayload>(&payload_json).ok()
    }
}

/// Wire payload encoded into the bearer token. Field names are short to
/// keep the token compact; this struct is never persisted.
#[derive(Serialize, Deserialize)]
struct TokenPayload {
    sid: String,
    tenant: String,
    user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ep: Option<SessionEpoch>,
    iat: i64,
    exp: i64,
}

#[async_trait]
impl SessionStore for SignedTokenSessionStore {
    async fn create_session(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
        lifetime: ChronoDuration,
    ) -> Result<SecretString, SessionStoreError> {
        // A non-positive lifetime would mint a token whose `exp <= iat`;
        // `lookup` then rejects it immediately, so the caller would get
        // `Ok(token)` for an already-dead session. Fail loud instead.
        if lifetime <= ChronoDuration::zero() {
            return Err(SessionStoreError::Backend(
                "session lifetime must be positive".into(),
            ));
        }
        let now = Utc::now();
        let expires_at = now
            .checked_add_signed(lifetime)
            .ok_or_else(|| SessionStoreError::Backend("session lifetime overflow".into()))?;
        let payload = TokenPayload {
            sid: Uuid::new_v4().to_string(),
            tenant: tenant_id.as_str().to_string(),
            user: user_id.as_str().to_string(),
            ep: self.session_epoch.clone(),
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
        };
        let payload_json = serde_json::to_vec(&payload)
            .map_err(|err| SessionStoreError::Backend(format!("encode token payload: {err}")))?;
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload_json);
        let signature = self.sign(&payload_b64);
        Ok(SecretString::from(format!("{payload_b64}.{signature}")))
    }

    async fn lookup(&self, candidate: &str) -> Result<Option<SessionRecord>, SessionStoreError> {
        let Some(payload) = self.verify(candidate) else {
            return Ok(None);
        };
        // RFC 7519 §4.1.4: a token must not be accepted after `exp`; the
        // expiry second itself is still valid.
        let now = Utc::now().timestamp();
        if payload.exp < now {
            return Ok(None);
        }
        // Honor server-side revocation (logout) within this process.
        if self.revoked.read().contains_key(&payload.sid) {
            return Ok(None);
        }
        // A malformed identity inside a validly-signed token is a
        // backend inconsistency, not an auth miss — surface it.
        let tenant_id = TenantId::new(&payload.tenant)
            .map_err(|err| SessionStoreError::Backend(format!("token tenant: {err}")))?;
        // Defense in depth atop the tenant-bound key: reject a
        // validly-parsed token whose tenant differs from this host's. (A
        // cross-tenant token cannot validly sign under the tenant-bound
        // key, so reaching here with a mismatch should be impossible — but
        // fail closed rather than stamp the host tenant onto a foreign
        // token if the key derivation ever changes.)
        if tenant_id.as_str() != self.tenant_id.as_str() {
            return Ok(None);
        }
        let user_id = UserId::new(&payload.user)
            .map_err(|err| SessionStoreError::Backend(format!("token user: {err}")))?;
        if self.session_epoch.as_ref().is_some_and(
            |epoch| !matches!(payload.ep.as_ref(), Some(payload_epoch) if payload_epoch == epoch),
        ) {
            return Ok(None);
        }
        let allowed = self
            .access_validator
            .has_session_access(&tenant_id, &user_id)
            .await
            .map_err(|err| SessionStoreError::Backend(err.to_string()))?;
        if !allowed {
            tracing::debug!(
                tenant_id = %tenant_id.as_str(),
                user_id = %user_id.as_str(),
                "signed WebUI session rejected because user no longer has active access"
            );
            return Ok(None);
        }
        if self.revoked.read().contains_key(&payload.sid) {
            return Ok(None);
        }
        let created_at = DateTime::<Utc>::from_timestamp(payload.iat, 0)
            .ok_or_else(|| SessionStoreError::Backend("token iat out of range".into()))?;
        let expires_at = DateTime::<Utc>::from_timestamp(payload.exp, 0)
            .ok_or_else(|| SessionStoreError::Backend("token exp out of range".into()))?;
        Ok(Some(SessionRecord {
            session_id: SessionId::new(payload.sid),
            tenant_id,
            user_id,
            created_at,
            expires_at,
        }))
    }

    async fn revoke(&self, candidate: &str) -> Result<(), SessionStoreError> {
        // Only a validly-signed, not-yet-expired token carries a session
        // id worth denying; a garbage or already-expired bearer has nothing
        // to revoke, so logout is a silent success.
        if let Some(payload) = self.verify(candidate) {
            let now = Utc::now().timestamp();
            if payload.exp <= now {
                return Ok(());
            }
            let mut guard = self.revoked.write();
            // Common case: O(1) insert. Only when the map reaches the cap do
            // we pay the O(n) expired-entry sweep (keeping the per-logout
            // write-lock hold time off the hot per-request `lookup` path in
            // the common case).
            if guard.len() >= MAX_REVOKED_ENTRIES {
                guard.retain(|_, exp| *exp > now);
                // Still at the cap with all-live entries → evict the one
                // closest to expiry so the set stays bounded.
                if guard.len() >= MAX_REVOKED_ENTRIES
                    && let Some(soonest) = guard
                        .iter()
                        .min_by_key(|(_, exp)| **exp)
                        .map(|(sid, _)| sid.clone())
                {
                    guard.remove(&soonest);
                }
            }
            guard.insert(payload.sid, payload.exp);
        }
        Ok(())
    }
}

/// `WebuiAuthenticator` that accepts a bearer recognized by EITHER the
/// session token or the env operator token. Keeping the env-bearer path
/// live means the existing scripted `Authorization: Bearer` workflow
/// keeps working while a browser SSO login mints a signed session token.
struct CompositeAuthenticator {
    session: Arc<dyn WebuiAuthenticator>,
    env_token: Arc<dyn WebuiAuthenticator>,
}

impl CompositeAuthenticator {
    fn new(session: Arc<dyn WebuiAuthenticator>, env_token: Arc<dyn WebuiAuthenticator>) -> Self {
        Self { session, env_token }
    }
}

#[async_trait]
impl WebuiAuthenticator for CompositeAuthenticator {
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        if let Some(auth) = self.env_token.authenticate(token).await {
            return Some(auth);
        }
        self.session.authenticate(token).await
    }

    fn mounts_operator_webui_config_routes(&self) -> bool {
        self.env_token.mounts_operator_webui_config_routes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{OAuthError, OAuthProviderName, OAuthUserProfile, UserDirectoryError};
    use secrecy::ExposeSecret;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;

    fn tenant() -> TenantId {
        TenantId::new("tenant-a").expect("tenant")
    }

    fn allow_all_access_validator() -> Arc<dyn SessionUserAccessValidator> {
        Arc::new(AllowAllAccessValidator)
    }

    fn signed_store(secret: &str) -> SignedTokenSessionStore {
        SignedTokenSessionStore::from_operator_secret(
            &SecretString::from(secret.to_string()),
            &tenant(),
            None,
            allow_all_access_validator(),
        )
    }

    fn signed_store_for(secret: &str, tenant_id: &TenantId) -> SignedTokenSessionStore {
        SignedTokenSessionStore::from_operator_secret(
            &SecretString::from(secret.to_string()),
            tenant_id,
            None,
            allow_all_access_validator(),
        )
    }

    #[test]
    fn session_epoch_validates_boundary_values() {
        assert_eq!(
            SessionEpoch::new("deploy-2026-07-02")
                .expect("valid epoch")
                .as_str(),
            "deploy-2026-07-02",
        );
        assert!(matches!(
            SessionEpoch::new(""),
            Err(SessionEpochError::Empty),
        ));
        assert!(matches!(
            SessionEpoch::new("deploy\n2026"),
            Err(SessionEpochError::ControlCharacter),
        ));
        assert!(matches!(
            SessionEpoch::new("x".repeat(SESSION_EPOCH_MAX_CHARS + 1)),
            Err(SessionEpochError::TooLong { .. }),
        ));
    }

    /// Mint a raw `{payload}.{sig}` token directly from a payload — used
    /// to craft tokens (expired, malformed identity) that
    /// `create_session` would refuse to produce.
    fn signed_raw(store: &SignedTokenSessionStore, payload: &TokenPayload) -> String {
        let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).expect("encode"));
        let signature = store.sign(&payload_b64);
        format!("{payload_b64}.{signature}")
    }

    struct ToggleAccessValidator {
        allowed: AtomicBool,
    }

    struct AllowAllAccessValidator;

    struct FailingAccessValidator;

    struct BlockingAccessValidator {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl SessionUserAccessValidator for AllowAllAccessValidator {
        async fn has_session_access(
            &self,
            _tenant_id: &TenantId,
            _user_id: &UserId,
        ) -> Result<bool, SessionUserAccessError> {
            Ok(true)
        }
    }

    #[async_trait]
    impl SessionUserAccessValidator for FailingAccessValidator {
        async fn has_session_access(
            &self,
            _tenant_id: &TenantId,
            _user_id: &UserId,
        ) -> Result<bool, SessionUserAccessError> {
            Err(SessionUserAccessError::Backend("store unavailable".into()))
        }
    }

    #[async_trait]
    impl SessionUserAccessValidator for BlockingAccessValidator {
        async fn has_session_access(
            &self,
            _tenant_id: &TenantId,
            _user_id: &UserId,
        ) -> Result<bool, SessionUserAccessError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(true)
        }
    }

    #[async_trait]
    impl SessionUserAccessValidator for ToggleAccessValidator {
        async fn has_session_access(
            &self,
            _tenant_id: &TenantId,
            _user_id: &UserId,
        ) -> Result<bool, SessionUserAccessError> {
            Ok(self.allowed.load(Ordering::SeqCst))
        }
    }

    #[tokio::test]
    async fn signed_token_round_trips_tenant_and_user() {
        let store = signed_store("operator-secret");
        let token = store
            .create_session(
                tenant(),
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let record = store
            .lookup(token.expose_secret())
            .await
            .expect("lookup")
            .expect("valid session");
        assert_eq!(record.tenant_id.as_str(), "tenant-a");
        assert_eq!(record.user_id.as_str(), "operator");
    }

    #[tokio::test]
    async fn tampered_or_wrong_key_token_is_rejected() {
        let store = signed_store("secret-a");
        let token = store
            .create_session(
                tenant(),
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let raw = token.expose_secret().to_string();

        let mut tampered = raw.clone();
        let first = tampered.remove(0);
        tampered.insert(0, if first == 'A' { 'B' } else { 'A' });
        assert!(store.lookup(&tampered).await.expect("lookup").is_none());

        let other = signed_store("secret-b");
        assert!(other.lookup(&raw).await.expect("lookup").is_none());

        assert!(store.lookup("not-a-token").await.expect("lookup").is_none());
    }

    #[tokio::test]
    async fn same_secret_different_tenant_tokens_are_rejected() {
        // Two serve instances sharing one operator secret but configured
        // for different tenants must NOT accept each other's session
        // tokens — otherwise a multi-tenant operator that reuses a secret
        // leaks cross-tenant access.
        let tenant_a = TenantId::new("tenant-a").expect("tenant");
        let tenant_b = TenantId::new("tenant-b").expect("tenant");
        let store_a = signed_store_for("shared-secret", &tenant_a);
        let store_b = signed_store_for("shared-secret", &tenant_b);

        let token = store_a
            .create_session(
                tenant_a.clone(),
                UserId::new("alice").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let raw = token.expose_secret().to_string();

        assert!(
            store_a.lookup(&raw).await.expect("lookup").is_some(),
            "the minting host must accept its own token",
        );
        assert!(
            store_b.lookup(&raw).await.expect("lookup").is_none(),
            "a session minted for tenant-a must not authenticate against a \
             same-secret host bound to tenant-b",
        );
    }

    #[tokio::test]
    async fn configured_session_epoch_rejects_stale_tokens() {
        let tenant_id = tenant();
        let store_epoch_1 = SignedTokenSessionStore::from_operator_secret(
            &SecretString::from("operator-secret".to_string()),
            &tenant_id,
            Some(SessionEpoch::new("epoch-1").expect("valid epoch")),
            allow_all_access_validator(),
        );
        let store_epoch_2 = SignedTokenSessionStore::from_operator_secret(
            &SecretString::from("operator-secret".to_string()),
            &tenant_id,
            Some(SessionEpoch::new("epoch-2").expect("valid epoch")),
            allow_all_access_validator(),
        );
        let token = store_epoch_1
            .create_session(
                tenant_id,
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let raw = token.expose_secret().to_string();

        assert!(
            store_epoch_1.lookup(&raw).await.expect("lookup").is_some(),
            "the minting epoch must accept its own token",
        );
        assert!(
            store_epoch_2.lookup(&raw).await.expect("lookup").is_none(),
            "changing the configured session epoch must force browsers through SSO again",
        );
    }

    #[tokio::test]
    async fn configured_session_epoch_rejects_legacy_tokens_without_epoch() {
        let tenant_id = tenant();
        let legacy_store = SignedTokenSessionStore::from_operator_secret(
            &SecretString::from("operator-secret".to_string()),
            &tenant_id,
            None,
            allow_all_access_validator(),
        );
        let epoch_store = SignedTokenSessionStore::from_operator_secret(
            &SecretString::from("operator-secret".to_string()),
            &tenant_id,
            Some(SessionEpoch::new("epoch-1").expect("valid epoch")),
            allow_all_access_validator(),
        );
        let token = legacy_store
            .create_session(
                tenant_id,
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let raw = token.expose_secret().to_string();

        assert!(
            legacy_store.lookup(&raw).await.expect("lookup").is_some(),
            "a pre-epoch store must still accept its own token",
        );
        assert!(
            epoch_store.lookup(&raw).await.expect("lookup").is_none(),
            "enabling a configured session epoch must reject legacy tokens with no epoch",
        );
    }

    #[tokio::test]
    async fn access_validator_can_invalidate_existing_signed_token() {
        let tenant_id = tenant();
        let validator = Arc::new(ToggleAccessValidator {
            allowed: AtomicBool::new(true),
        });
        let access_validator: Arc<dyn SessionUserAccessValidator> = validator.clone();
        let store = SignedTokenSessionStore::from_operator_secret(
            &SecretString::from("operator-secret".to_string()),
            &tenant_id,
            None,
            access_validator,
        );
        let token = store
            .create_session(
                tenant_id,
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let raw = token.expose_secret().to_string();
        assert!(store.lookup(&raw).await.expect("lookup").is_some());

        validator.allowed.store(false, Ordering::SeqCst);
        assert!(
            store.lookup(&raw).await.expect("lookup").is_none(),
            "the same validly signed token must be rejected once the host access validator denies it",
        );
    }

    #[tokio::test]
    async fn access_validator_backend_error_is_returned_from_lookup() {
        let tenant_id = tenant();
        let store = SignedTokenSessionStore::from_operator_secret(
            &SecretString::from("operator-secret".to_string()),
            &tenant_id,
            None,
            Arc::new(FailingAccessValidator),
        );
        let token = store
            .create_session(
                tenant_id,
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let error = store
            .lookup(token.expose_secret())
            .await
            .expect_err("validator backend errors must surface");

        assert!(
            matches!(error, SessionStoreError::Backend(ref message) if message.contains("store unavailable")),
            "lookup should preserve the validator backend failure, got: {error:?}",
        );
    }

    #[tokio::test]
    async fn expired_token_is_rejected() {
        let store = signed_store("operator-secret");
        let now = Utc::now().timestamp();
        let token = signed_raw(
            &store,
            &TokenPayload {
                sid: "session-1".to_string(),
                tenant: "tenant-a".to_string(),
                user: "operator".to_string(),
                ep: None,
                iat: now - 100,
                exp: now - 10,
            },
        );
        assert!(store.lookup(&token).await.expect("lookup").is_none());
    }

    #[tokio::test]
    async fn revoke_invalidates_a_minted_token() {
        // The logout/revoke contract: a token valid before `revoke` must
        // be rejected after it, so `POST /auth/logout` is not a lie.
        let store = signed_store("operator-secret");
        let token = store
            .create_session(
                tenant(),
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let raw = token.expose_secret().to_string();
        assert!(store.lookup(&raw).await.expect("lookup").is_some());

        store.revoke(&raw).await.expect("revoke");
        assert!(
            store.lookup(&raw).await.expect("lookup").is_none(),
            "a revoked token must no longer authenticate"
        );
    }

    #[tokio::test]
    async fn revoke_during_access_validation_is_observed_before_authentication() {
        let tenant_id = tenant();
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let store = Arc::new(SignedTokenSessionStore::from_operator_secret(
            &SecretString::from("operator-secret".to_string()),
            &tenant_id,
            None,
            Arc::new(BlockingAccessValidator {
                started: started.clone(),
                release: release.clone(),
            }),
        ));
        let token = store
            .create_session(
                tenant_id,
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let raw = token.expose_secret().to_string();
        let lookup_store = store.clone();
        let lookup_raw = raw.clone();
        let lookup = tokio::spawn(async move { lookup_store.lookup(&lookup_raw).await });

        started.notified().await;
        store
            .revoke(&raw)
            .await
            .expect("revoke while lookup awaits");
        release.notify_one();

        let result = lookup.await.expect("lookup task").expect("lookup");
        assert!(
            result.is_none(),
            "lookup must re-check revocation after access validation awaits",
        );
    }

    #[tokio::test]
    async fn create_session_rejects_non_positive_lifetime() {
        let store = signed_store("operator-secret");
        for lifetime in [ChronoDuration::zero(), ChronoDuration::seconds(-1)] {
            let err = store
                .create_session(tenant(), UserId::new("operator").expect("user"), lifetime)
                .await
                .expect_err("a non-positive lifetime must error, not mint a dead token");
            assert!(matches!(err, SessionStoreError::Backend(_)));
        }
    }

    #[tokio::test]
    async fn create_session_returns_error_on_lifetime_overflow() {
        let store = signed_store("operator-secret");
        let err = store
            .create_session(
                tenant(),
                UserId::new("operator").expect("user"),
                ChronoDuration::MAX,
            )
            .await
            .expect_err("a lifetime that overflows the expiry instant must error");
        assert!(matches!(err, SessionStoreError::Backend(_)));
    }

    #[tokio::test]
    async fn lookup_surfaces_backend_error_on_malformed_tenant() {
        let store = signed_store("operator-secret");
        let now = Utc::now().timestamp();
        let token = signed_raw(
            &store,
            &TokenPayload {
                sid: "session-1".to_string(),
                tenant: String::new(),
                user: "operator".to_string(),
                ep: None,
                iat: now,
                exp: now + 3600,
            },
        );
        let err = store
            .lookup(&token)
            .await
            .expect_err("malformed tenant in a signed token must surface an error");
        assert!(matches!(err, SessionStoreError::Backend(_)));
    }

    #[tokio::test]
    async fn lookup_surfaces_backend_error_on_malformed_user() {
        // Symmetric to the malformed-tenant case: a validly-signed token
        // with this host's tenant but an empty (invalid) user id is a
        // backend inconsistency, not a silent auth miss. A regression that
        // swallowed it into `Ok(None)` (or panicked) must fail here.
        let store = signed_store("operator-secret");
        let now = Utc::now().timestamp();
        let token = signed_raw(
            &store,
            &TokenPayload {
                sid: "session-1".to_string(),
                tenant: tenant().as_str().to_string(),
                user: String::new(),
                ep: None,
                iat: now,
                exp: now + 3600,
            },
        );
        let err = store
            .lookup(&token)
            .await
            .expect_err("malformed user in a signed token must surface an error");
        assert!(matches!(err, SessionStoreError::Backend(_)));
    }

    /// Stub authenticator that recognizes exactly one token.
    struct OneToken {
        token: &'static str,
        user: &'static str,
        operator: bool,
    }

    #[async_trait]
    impl WebuiAuthenticator for OneToken {
        async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
            if token == self.token {
                let user = UserId::new(self.user).expect("valid user id");
                Some(if self.operator {
                    WebuiAuthentication::operator(user)
                } else {
                    WebuiAuthentication::user(user)
                })
            } else {
                None
            }
        }

        fn mounts_operator_webui_config_routes(&self) -> bool {
            self.operator
        }
    }

    #[tokio::test]
    async fn composite_accepts_either_source_and_rejects_unknown() {
        let composite = CompositeAuthenticator::new(
            Arc::new(OneToken {
                token: "session-tok",
                user: "alice@example.com",
                operator: false,
            }),
            Arc::new(OneToken {
                token: "env-tok",
                user: "operator",
                operator: true,
            }),
        );

        assert_eq!(
            composite
                .authenticate("session-tok")
                .await
                .unwrap()
                .user_id
                .as_str(),
            "alice@example.com"
        );
        assert_eq!(
            composite
                .authenticate("env-tok")
                .await
                .unwrap()
                .user_id
                .as_str(),
            "operator"
        );
        assert!(composite.authenticate("nope").await.is_none());
    }

    #[tokio::test]
    async fn composite_marks_only_env_token_as_operator_capable() {
        let composite = CompositeAuthenticator::new(
            Arc::new(OneToken {
                token: "session-tok",
                user: "alice@example.com",
                operator: false,
            }),
            Arc::new(OneToken {
                token: "env-tok",
                user: "operator",
                operator: true,
            }),
        );

        let session = composite
            .authenticate("session-tok")
            .await
            .expect("session token authenticates");
        assert_eq!(session.user_id.as_str(), "alice@example.com");
        assert!(!session.capabilities.operator_webui_config);

        let env = composite
            .authenticate("env-tok")
            .await
            .expect("env token authenticates");
        assert_eq!(env.user_id.as_str(), "operator");
        assert!(env.capabilities.operator_webui_config);
        assert!(composite.mounts_operator_webui_config_routes());
    }

    /// Minimal host-supplied directory for the builder wiring tests.
    struct StubUserDirectory;

    #[async_trait]
    impl UserDirectory for StubUserDirectory {
        async fn resolve(
            &self,
            _provider: &OAuthProviderName,
            _profile: &OAuthUserProfile,
        ) -> Result<UserId, UserDirectoryError> {
            UserId::new("resolved-user").map_err(|e| UserDirectoryError::Backend(e.to_string()))
        }
    }

    /// Minimal `OAuthProvider` for the builder wiring tests — the
    /// builder only stores the provider list, so the URL / exchange
    /// methods are never invoked here.
    struct StubProvider(OAuthProviderName);

    #[async_trait]
    impl OAuthProvider for StubProvider {
        fn name(&self) -> &OAuthProviderName {
            &self.0
        }
        fn authorization_url(&self, _callback_url: &str, _state: &str, _challenge: &str) -> String {
            "https://provider.example/authorize".to_string()
        }
        async fn exchange_code(
            &self,
            _code: &str,
            _callback_url: &str,
            _verifier: &str,
        ) -> Result<OAuthUserProfile, OAuthError> {
            unreachable!("exchange_code is not exercised by the wiring test")
        }
    }

    fn login_config(providers: Vec<Arc<dyn OAuthProvider>>) -> SignedSessionLoginConfig {
        SignedSessionLoginConfig {
            tenant_id: tenant(),
            user_directory: Arc::new(StubUserDirectory),
            operator_secret: SecretString::from("operator-secret".to_string()),
            session_epoch: None,
            session_user_access_validator: allow_all_access_validator(),
            base_url: "https://app.example".to_string(),
            providers,
            env_authenticator: Arc::new(OneToken {
                token: "env-tok",
                user: "operator",
                operator: true,
            }),
        }
    }

    #[test]
    fn build_signed_session_login_returns_none_when_no_providers() {
        assert!(build_signed_session_login(login_config(Vec::new())).is_none());
    }

    #[test]
    fn build_signed_session_login_returns_wiring_when_a_provider_is_present() {
        let provider: Arc<dyn OAuthProvider> = Arc::new(StubProvider(
            OAuthProviderName::new("google").expect("name"),
        ));
        assert!(build_signed_session_login(login_config(vec![provider])).is_some());
    }
}
