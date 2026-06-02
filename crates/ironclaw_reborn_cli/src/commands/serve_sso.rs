//! WebChat v2 SSO login wiring for `ironclaw-reborn serve`.
//!
//! Compiled under the `webui-v2-beta` feature (the same feature that
//! compiles the `serve` command). When a Google/GitHub OAuth provider
//! is configured via env (`IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID=…`,
//! `IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID=…`),
//! this builds the host-owned `webui_v2_auth_router` and the
//! authenticator the protected v2 routes need so the SSO-minted session
//! bearer is accepted alongside the env operator token. The serve
//! command merges the mount through
//! `WebuiServeConfig::with_public_route_mount`.
//!
//! Sessions are **stateless, HMAC-signed bearer tokens**
//! ([`SignedTokenSessionStore`]): the token carries the tenant/user/
//! expiry and is signed with a key derived from the operator secret, so
//! it survives restarts without any session store to persist — fitting
//! the single-operator `serve` deployment shape (and mirroring the
//! crate's existing stateless `OidcAuthenticator`). Server-side revoke
//! is best-effort (logout clears the client token); a multi-user
//! deployment supplies a durable DB-backed `SessionStore` instead.
//!
//! Every successful login is mapped to the single operator identity
//! (`IRONCLAW_REBORN_WEBUI_USER_ID`) via [`FixedUserDirectory`] so chat
//! turns run under the same owner the serve runtime is pinned to.

use std::env;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use ironclaw_reborn_composition::WebuiAuthenticator;
use ironclaw_reborn_composition::host_api::{TenantId, UserId};
use ironclaw_reborn_webui_ingress::{
    GitHubOAuthConfig, GitHubProvider, GoogleOAuthConfig, GoogleProvider, OAuthProvider,
    OAuthProviderName, OAuthRouterConfig, OAuthUserProfile, PublicRouteMount, SessionAuthenticator,
    SessionId, SessionRecord, SessionStore, SessionStoreError, UserDirectory, UserDirectoryError,
    webui_v2_auth_router,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// The SSO login surface: the public route mount to merge into
/// `webui_v2_app`, plus the authenticator the protected v2 routes must
/// use so the SSO-minted session bearer authenticates.
pub(crate) struct OAuthLoginWiring {
    pub(crate) mount: PublicRouteMount,
    pub(crate) authenticator: Arc<dyn WebuiAuthenticator>,
}

/// Stateless `SessionStore` whose "record" is the cryptographic
/// signature itself: the bearer token carries the tenant/user/expiry,
/// HMAC-SHA256-signed with a key derived from the operator secret.
///
/// `lookup` verifies the signature (constant-time, via `hmac`'s
/// `verify_slice`) and the expiry — no persistence, so tokens survive a
/// restart as long as the operator secret is stable. `revoke` is a
/// best-effort no-op (the trait default): a stateless token cannot be
/// invalidated server-side before expiry without a denylist, which the
/// single-operator deployment does not need (logout clears the client
/// token). A multi-user deployment swaps in a durable store.
struct SignedTokenSessionStore {
    key: Vec<u8>,
}

impl SignedTokenSessionStore {
    /// Derive the HMAC key from the operator secret, domain-separated so
    /// the session-signing key never collides with another use of the
    /// same secret.
    fn from_operator_secret(secret: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"ironclaw-reborn-webui-session-v1::");
        hasher.update(secret.as_bytes());
        Self {
            key: hasher.finalize().to_vec(),
        }
    }

    fn sign(&self, payload_b64: &str) -> String {
        let mut mac =
            HmacSha256::new_from_slice(&self.key).expect("HMAC accepts a key of any length"); // safety: HMAC-SHA256 has no key-length constraint
        mac.update(payload_b64.as_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
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
        let Some((payload_b64, signature_b64)) = candidate.split_once('.') else {
            return Ok(None);
        };
        // Verify the signature in constant time before trusting any
        // bytes of the payload.
        let Ok(signature) = URL_SAFE_NO_PAD.decode(signature_b64) else {
            return Ok(None);
        };
        let mut mac =
            HmacSha256::new_from_slice(&self.key).expect("HMAC accepts a key of any length"); // safety: HMAC-SHA256 has no key-length constraint
        mac.update(payload_b64.as_bytes());
        if mac.verify_slice(&signature).is_err() {
            return Ok(None);
        }
        let Ok(payload_json) = URL_SAFE_NO_PAD.decode(payload_b64) else {
            return Ok(None);
        };
        let Ok(payload) = serde_json::from_slice::<TokenPayload>(&payload_json) else {
            return Ok(None);
        };
        let now = Utc::now().timestamp();
        if payload.exp <= now {
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
}

/// Authenticator that accepts a bearer recognized by EITHER the SSO
/// session token or the env operator token.
///
/// Keeping the env-bearer path live means the existing
/// `IRONCLAW_REBORN_WEBUI_TOKEN` curl / scripted workflow keeps working
/// while a browser SSO login mints a signed session token that
/// authenticates through the same [`SignedTokenSessionStore`].
struct CompositeAuthenticator {
    session: Arc<dyn WebuiAuthenticator>,
    env_token: Arc<dyn WebuiAuthenticator>,
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
/// operator `UserId` the serve command pins as the runtime owner
/// (`IRONCLAW_REBORN_WEBUI_USER_ID`).
///
/// The standalone serve runtime fixes its owner at startup, while the v2
/// facade derives each thread's scope from the authenticated caller. If
/// those diverge, every turn fails with "thread scope owner does not
/// match the loop run actor". Mapping all logins to the operator keeps
/// them aligned — the same single-operator model the env-bearer token
/// already uses, just unlocked through an OAuth handshake. The full
/// provider flow still runs (redirect, code exchange, profile fetch);
/// only the final identity mapping is fixed. A multi-user deployment
/// supplies a real DB-backed `UserDirectory` instead.
struct FixedUserDirectory {
    user_id: UserId,
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

/// Build the SSO login surface from environment configuration.
///
/// `owner_user_id` is the operator identity the serve command pins as
/// the runtime owner; every successful login maps to it (see
/// [`FixedUserDirectory`]). `operator_secret` keys the session-token
/// HMAC (see [`SignedTokenSessionStore`]).
///
/// Returns `Ok(None)` when no OAuth provider is enabled (Google or
/// GitHub), in which case the caller keeps the plain env-bearer
/// authenticator and mounts no public login routes.
pub(crate) fn build_oauth_login(
    tenant_id: TenantId,
    owner_user_id: UserId,
    operator_secret: &str,
    base_url: String,
    env_authenticator: Arc<dyn WebuiAuthenticator>,
) -> anyhow::Result<Option<OAuthLoginWiring>> {
    let providers = oauth_providers_from_env()?;

    // Nothing configured → keep the plain env-bearer listener and mount
    // no public login routes.
    if providers.is_empty() {
        return Ok(None);
    }

    let session_store: Arc<dyn SessionStore> = Arc::new(
        SignedTokenSessionStore::from_operator_secret(operator_secret),
    );
    let session_authenticator: Arc<dyn WebuiAuthenticator> =
        Arc::new(SessionAuthenticator::new(session_store.clone()));

    let user_directory: Arc<dyn UserDirectory> = Arc::new(FixedUserDirectory {
        user_id: owner_user_id,
    });
    let config = OAuthRouterConfig::new(
        tenant_id,
        session_store,
        user_directory,
        providers,
        base_url,
    );

    let mount = webui_v2_auth_router(config);
    let authenticator: Arc<dyn WebuiAuthenticator> = Arc::new(CompositeAuthenticator {
        session: session_authenticator,
        env_token: env_authenticator,
    });

    Ok(Some(OAuthLoginWiring {
        mount,
        authenticator,
    }))
}

/// Collect the configured OAuth providers from env. A provider is
/// opted in by setting its `IRONCLAW_REBORN_WEBUI_*_CLIENT_ID`; the
/// matching `*_CLIENT_SECRET` is read alongside (both are required for
/// the real code exchange to succeed, but the button surfaces as soon
/// as the client id is present so the login UI can be exercised
/// locally).
///
/// These WebChat-login vars live in the `IRONCLAW_REBORN_WEBUI_*`
/// namespace — the same one as `IRONCLAW_REBORN_WEBUI_TOKEN` /
/// `IRONCLAW_REBORN_WEBUI_USER_ID` — deliberately separate from the
/// bare `GOOGLE_CLIENT_ID` / `IRONCLAW_REBORN_GOOGLE_*` vars that the
/// product-auth credential-connection flow reads. Sharing the bare
/// names would couple two unrelated surfaces: setting `GOOGLE_CLIENT_ID`
/// just to enable login would also activate the product-auth Google
/// resolver, which hard-errors when its required redirect URI is
/// absent and would take down every `ironclaw-reborn` command. The
/// distinct namespace keeps SSO login and product-auth independently
/// configurable. (The browser-user *login* client and the product
/// *credential* client are usually distinct OAuth clients anyway —
/// different registered redirect URIs.)
fn oauth_providers_from_env() -> anyhow::Result<Vec<Arc<dyn OAuthProvider>>> {
    let mut providers: Vec<Arc<dyn OAuthProvider>> = Vec::new();

    if let Some(client_id) = non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_ID") {
        let provider = GoogleProvider::new(GoogleOAuthConfig {
            client_id,
            client_secret: SecretString::from(
                env::var("IRONCLAW_REBORN_WEBUI_GOOGLE_CLIENT_SECRET").unwrap_or_default(),
            ),
            allowed_hd: non_empty_env("IRONCLAW_REBORN_WEBUI_GOOGLE_ALLOWED_HD"),
        })
        .context("failed to build Google OAuth provider")?;
        providers.push(Arc::new(provider));
    }

    if let Some(client_id) = non_empty_env("IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_ID") {
        let provider = GitHubProvider::new(GitHubOAuthConfig {
            client_id,
            client_secret: SecretString::from(
                env::var("IRONCLAW_REBORN_WEBUI_GITHUB_CLIENT_SECRET").unwrap_or_default(),
            ),
        })
        .context("failed to build GitHub OAuth provider")?;
        providers.push(Arc::new(provider));
    }

    Ok(providers)
}

/// Read an env var, returning `None` when it is unset or blank.
fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|raw| !raw.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn tenant() -> TenantId {
        TenantId::new("tenant-a").expect("tenant")
    }

    #[tokio::test]
    async fn signed_token_round_trips_tenant_and_user() {
        let store = SignedTokenSessionStore::from_operator_secret("operator-secret");
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
        let store = SignedTokenSessionStore::from_operator_secret("secret-a");
        let token = store
            .create_session(
                tenant(),
                UserId::new("operator").expect("user"),
                ChronoDuration::hours(1),
            )
            .await
            .expect("create");
        let raw = token.expose_secret().to_string();

        // Flipping a payload byte breaks the signature.
        let mut tampered = raw.clone();
        let first = tampered.remove(0);
        tampered.insert(0, if first == 'A' { 'B' } else { 'A' });
        assert!(store.lookup(&tampered).await.expect("lookup").is_none());

        // A store built from a different operator secret rejects the
        // token (signature mismatch).
        let other = SignedTokenSessionStore::from_operator_secret("secret-b");
        assert!(other.lookup(&raw).await.expect("lookup").is_none());

        // Garbage / non-token input is rejected, not an error.
        assert!(store.lookup("not-a-token").await.expect("lookup").is_none());
    }

    #[tokio::test]
    async fn expired_token_is_rejected() {
        let store = SignedTokenSessionStore::from_operator_secret("operator-secret");
        // A negative lifetime puts `exp` in the past.
        let token = store
            .create_session(
                tenant(),
                UserId::new("operator").expect("user"),
                ChronoDuration::seconds(-1),
            )
            .await
            .expect("create");
        assert!(
            store
                .lookup(token.expose_secret())
                .await
                .expect("lookup")
                .is_none()
        );
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
        let composite = CompositeAuthenticator {
            session: Arc::new(OneToken {
                token: "session-tok",
                user: "alice@example.com",
            }),
            env_token: Arc::new(OneToken {
                token: "env-tok",
                user: "operator",
            }),
        };

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
        let dir = FixedUserDirectory {
            user_id: UserId::new("operator").expect("user id"),
        };
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
}
