//! Stateless, HMAC-signed session login wiring — the host-owned,
//! production-suitable counterpart to the dev-only
//! [`InMemorySessionStore`](crate::session::InMemorySessionStore).
//!
//! [`build_signed_session_login`] assembles the pieces a single-operator
//! host (the standalone `ironclaw-reborn serve` binary) needs to mount
//! the OAuth login surface, so the CLI only has to supply env/boot
//! config (provider client ids/secrets, base URL, operator identity +
//! secret) and call the builder — it does not own the auth/session
//! model. That keeps the rule from this crate's guardrails intact:
//! `WebuiAuthenticator` / `SessionStore` implementations live here, not
//! in the command crate.
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
//! - [`FixedUserDirectory`] — maps every successful login to the single
//!   operator identity, keeping the v2 thread scope aligned with the
//!   serve runtime's pinned owner.
//! - [`CompositeAuthenticator`] — accepts EITHER a minted session token
//!   OR the host's existing env-bearer operator token.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_reborn_composition::WebuiAuthenticator;
use parking_lot::RwLock;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::{
    OAuthProvider, OAuthRouterConfig, PublicRouteMount, UserDirectory, UserDirectoryError,
    webui_v2_auth_router,
};
use crate::auth::{OAuthProviderName, OAuthUserProfile};
use crate::session::{
    SessionAuthenticator, SessionId, SessionRecord, SessionStore, SessionStoreError,
};

type HmacSha256 = Hmac<Sha256>;

/// Host-supplied input to [`build_signed_session_login`].
pub struct SignedSessionLoginConfig {
    /// Host-trusted installation tenant; never browser-influenced.
    pub tenant_id: TenantId,
    /// The single operator identity every login maps to (see
    /// [`FixedUserDirectory`]); must match the serve runtime's pinned
    /// owner so authenticated turns run under the expected owner.
    pub operator_user_id: UserId,
    /// Operator secret the session-token HMAC key is derived from. The
    /// same value typically backs the env-bearer authenticator.
    pub operator_secret: SecretString,
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

    let session_store: Arc<dyn SessionStore> = Arc::new(
        SignedTokenSessionStore::from_operator_secret(&config.operator_secret),
    );
    let session_authenticator: Arc<dyn WebuiAuthenticator> =
        Arc::new(SessionAuthenticator::new(session_store.clone()));
    let user_directory: Arc<dyn UserDirectory> =
        Arc::new(FixedUserDirectory::new(config.operator_user_id));

    let router_config = OAuthRouterConfig::new(
        config.tenant_id,
        session_store,
        user_directory,
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

/// Stateless `SessionStore` whose "record" is the cryptographic
/// signature itself (HMAC-SHA256 over the base64url payload), plus a
/// process-local denylist that makes `revoke` (logout) effective.
struct SignedTokenSessionStore {
    key: Vec<u8>,
    /// Revoked session ids → their expiry (unix seconds). Pruned lazily
    /// on each `revoke` so the set stays bounded by "live revoked
    /// sessions", never "every session ever revoked".
    revoked: RwLock<HashMap<String, i64>>,
}

impl SignedTokenSessionStore {
    /// Derive the HMAC key from the operator secret, domain-separated so
    /// the session-signing key never collides with another use of the
    /// same secret.
    fn from_operator_secret(secret: &SecretString) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"ironclaw-reborn-webui-session-v1::");
        hasher.update(secret.expose_secret().as_bytes());
        Self {
            key: hasher.finalize().to_vec(),
            revoked: RwLock::new(HashMap::new()),
        }
    }

    fn sign(&self, payload_b64: &str) -> String {
        let mut mac =
            HmacSha256::new_from_slice(&self.key).expect("HMAC accepts a key of any length"); // safety: HMAC-SHA256 has no key-length constraint
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
        let mut mac =
            HmacSha256::new_from_slice(&self.key).expect("HMAC accepts a key of any length"); // safety: HMAC-SHA256 has no key-length constraint
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
        let user_id = UserId::new(&payload.user)
            .map_err(|err| SessionStoreError::Backend(format!("token user: {err}")))?;
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
        // id worth denying; a garbage or expired bearer has nothing to
        // revoke, so logout is a silent success. Prune expired denylist
        // entries on the way so the set stays bounded.
        if let Some(payload) = self.verify(candidate) {
            let now = Utc::now().timestamp();
            let mut guard = self.revoked.write();
            guard.retain(|_, exp| *exp > now);
            if payload.exp > now {
                guard.insert(payload.sid, payload.exp);
            }
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
    async fn authenticate(&self, token: &str) -> Option<UserId> {
        if let Some(user) = self.session.authenticate(token).await {
            return Some(user);
        }
        self.env_token.authenticate(token).await
    }
}

/// `UserDirectory` that maps EVERY successful login to the single
/// operator `UserId` the serve command pins as the runtime owner.
///
/// The standalone serve runtime fixes its owner at startup, while the v2
/// facade derives each thread's scope from the authenticated caller. If
/// those diverge, every turn fails with "thread scope owner does not
/// match the loop run actor". Mapping all logins to the operator keeps
/// them aligned — the same single-operator model the env-bearer token
/// already uses, unlocked through an OAuth handshake. The full provider
/// flow still runs (redirect, code exchange, profile fetch); only the
/// final identity mapping is fixed. A multi-user deployment supplies a
/// real DB-backed `UserDirectory` instead.
struct FixedUserDirectory {
    user_id: UserId,
}

impl FixedUserDirectory {
    fn new(user_id: UserId) -> Self {
        Self { user_id }
    }
}

#[async_trait]
impl UserDirectory for FixedUserDirectory {
    async fn resolve(
        &self,
        _provider: &OAuthProviderName,
        _profile: &OAuthUserProfile,
    ) -> Result<UserId, UserDirectoryError> {
        Ok(self.user_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::OAuthError;
    use secrecy::ExposeSecret;

    fn tenant() -> TenantId {
        TenantId::new("tenant-a").expect("tenant")
    }

    fn signed_store(secret: &str) -> SignedTokenSessionStore {
        SignedTokenSessionStore::from_operator_secret(&SecretString::from(secret.to_string()))
    }

    /// Mint a raw `{payload}.{sig}` token directly from a payload — used
    /// to craft tokens (expired, malformed identity) that
    /// `create_session` would refuse to produce.
    fn signed_raw(store: &SignedTokenSessionStore, payload: &TokenPayload) -> String {
        let payload_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).expect("encode"));
        let signature = store.sign(&payload_b64);
        format!("{payload_b64}.{signature}")
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
    async fn expired_token_is_rejected() {
        let store = signed_store("operator-secret");
        let now = Utc::now().timestamp();
        let token = signed_raw(
            &store,
            &TokenPayload {
                sid: "session-1".to_string(),
                tenant: "tenant-a".to_string(),
                user: "operator".to_string(),
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

    /// Stub authenticator that recognizes exactly one token.
    struct OneToken {
        token: &'static str,
        user: &'static str,
    }

    #[async_trait]
    impl WebuiAuthenticator for OneToken {
        async fn authenticate(&self, token: &str) -> Option<UserId> {
            if token == self.token {
                Some(UserId::new(self.user).expect("valid user id"))
            } else {
                None
            }
        }
    }

    #[tokio::test]
    async fn composite_accepts_either_source_and_rejects_unknown() {
        let composite = CompositeAuthenticator::new(
            Arc::new(OneToken {
                token: "session-tok",
                user: "alice@example.com",
            }),
            Arc::new(OneToken {
                token: "env-tok",
                user: "operator",
            }),
        );

        assert_eq!(
            composite
                .authenticate("session-tok")
                .await
                .unwrap()
                .as_str(),
            "alice@example.com"
        );
        assert_eq!(
            composite.authenticate("env-tok").await.unwrap().as_str(),
            "operator"
        );
        assert!(composite.authenticate("nope").await.is_none());
    }

    #[tokio::test]
    async fn fixed_user_directory_maps_every_login_to_the_operator() {
        let dir = FixedUserDirectory::new(UserId::new("operator").expect("user id"));
        let provider = OAuthProviderName::new("google").expect("provider");
        let profile = OAuthUserProfile {
            provider_user_id: "g-sub-999".to_string(),
            email: Some("someone.else@example.com".to_string()),
            email_verified: true,
            display_name: Some("Someone Else".to_string()),
        };
        let resolved = dir.resolve(&provider, &profile).await.expect("resolve");
        assert_eq!(resolved.as_str(), "operator");
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
            operator_user_id: UserId::new("operator").expect("user"),
            operator_secret: SecretString::from("operator-secret".to_string()),
            base_url: "https://app.example".to_string(),
            providers,
            env_authenticator: Arc::new(OneToken {
                token: "env-tok",
                user: "operator",
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
