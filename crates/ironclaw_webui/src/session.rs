//! Session-backed `WebuiAuthenticator` for the IronClaw WebChat v2
//! gateway.
//!
//! A session is the opaque bearer token the browser presents back on
//! every request after a successful login. The built-in login paths mint
//! signed session tokens via [`SignedTokenSessionStore`].
//!
//! The built-in production store is the signed-token store in
//! `signed_session_login`.

use std::sync::Arc;

use crate::{
    WebuiAuthentication, WebuiAuthenticator, signed_session_login::SignedTokenSessionStore,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{TenantId, UserId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Non-secret session identifier — a UUID stamped at creation and
/// safe to log, render in audit trails, or surface to operators. The
/// bearer token returned by [`SignedTokenSessionStore::create_session`] is a
/// SEPARATE secret value, returned wrapped in [`SecretString`], and
/// is the signed token presented by the browser. The record below carries
/// only this non-secret id, never the bearer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Persisted session record. Carries non-secret metadata only — the
/// bearer token returned by [`SignedTokenSessionStore::create_session`] is
/// deliberately ABSENT from this struct so `Debug` / `Serialize` impls cannot
/// accidentally surface live bearer material. The non-secret [`SessionId`] is
/// a UUID stamped at creation; safe to log and audit-trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: SessionId,
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Whether this session carries the single-trusted-operator capability
    /// (`WebuiAuthentication::operator`, `WebUiV2Capabilities::operator_webui_config`).
    /// Stamped once at mint time by the caller of [`SignedTokenSessionStore::create_session`]
    /// — provenance-based, never re-derived from the bearer at validation time —
    /// per the invariant that only a token verified against the host's
    /// operator-capable authenticator (the raw `Authorization: Bearer` env
    /// token, or the CLI's `/login?token=` link that verifies against the same
    /// authenticator) may mint an operator session. SSO/OAuth and any other
    /// multi-user login path must always pass `false`.
    ///
    /// `#[serde(default)]` so a pre-existing session record persisted before
    /// this field existed (or a `create_session` call site that hasn't been
    /// updated) deserializes to `false` — fails closed to non-operator rather
    /// than accidentally granting escalation.
    #[serde(default)]
    pub operator: bool,
}

impl SessionRecord {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

/// Errors raised by signed session operations.
#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("session not found")]
    NotFound,
    #[error("session backend failure: {0}")]
    Backend(String),
}

/// `WebuiAuthenticator` impl that resolves the bearer token to a
/// stored session, checking expiry against the wall clock.
#[derive(Clone)]
pub struct SessionAuthenticator {
    store: Arc<SignedTokenSessionStore>,
}

impl SessionAuthenticator {
    pub fn new(store: Arc<SignedTokenSessionStore>) -> Self {
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
    async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
        // The `WebuiAuthenticator` contract is `Option<WebuiAuthentication>` —
        // every failure must collapse to `None` so the gateway
        // emits a generic 401 and never leaks the reason to the
        // client. But "session not found" (auth miss) and
        // "backend errored" (infra outage) are not the same event
        // for OPERATORS: a DB outage silently turning into 401s
        // makes production auth failures impossible to diagnose.
        // Match explicitly so backend errors are logged at `warn!`
        // and auth misses stay quiet.
        let record = match self.store.lookup(token).await {
            Ok(Some(record)) => record,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(
                    target = "ironclaw::webui_ingress::session",
                    error = %error,
                    "session store lookup failed; treating bearer as unauthenticated. \
                     Operators: this is a backend/infra fault, not an auth miss — \
                     investigate the signed session store.",
                );
                return None;
            }
        };
        if record.is_expired(Utc::now()) {
            tracing::debug!(
                target = "ironclaw::webui_ingress::session",
                user = %record.user_id,
                session_id = %record.session_id,
                "rejecting expired session",
            );
            return None;
        }
        // Never re-derive operator-ness from the bearer; only stamp what
        // was recorded at mint time (see SessionRecord::operator doc).
        if record.operator {
            Some(WebuiAuthentication::operator(record.user_id))
        } else {
            Some(WebuiAuthentication::user(record.user_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use secrecy::ExposeSecret;
    use secrecy::SecretString;

    fn tenant() -> TenantId {
        TenantId::new("tenant-a").expect("tenant")
    }
    fn user() -> UserId {
        UserId::new("alice").expect("user")
    }
    fn store() -> Arc<SignedTokenSessionStore> {
        crate::signed_session_store(&SecretString::from("test-session-secret"), &tenant())
    }

    #[tokio::test]
    async fn create_then_lookup_returns_session() {
        let store = store();
        let token = store
            .create_session(tenant(), user(), ChronoDuration::hours(1), false)
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
    async fn unknown_token_is_rejected() {
        let auth = SessionAuthenticator::new(store());
        assert!(auth.authenticate("nonexistent-token").await.is_none());
    }

    #[tokio::test]
    async fn live_session_resolves_to_caller_user_id() {
        let store = store();
        let token = store
            .create_session(tenant(), user(), ChronoDuration::hours(1), false)
            .await
            .expect("create");
        let auth = SessionAuthenticator::new(store);
        let resolved = auth
            .authenticate(token.expose_secret())
            .await
            .expect("authenticated");
        assert_eq!(resolved.user_id.as_str(), "alice");
        assert!(!resolved.capabilities.operator_webui_config);
    }

    // operator = true (webui-token-authenticated) must resolve to
    // WebuiAuthentication::operator, not just ::user.
    #[tokio::test]
    async fn session_minted_as_operator_resolves_to_operator_capabilities() {
        let store = store();
        let token = store
            .create_session(tenant(), user(), ChronoDuration::hours(1), true)
            .await
            .expect("create");
        let auth = SessionAuthenticator::new(store);
        let resolved = auth
            .authenticate(token.expose_secret())
            .await
            .expect("authenticated");
        assert_eq!(resolved.user_id.as_str(), "alice");
        assert!(
            resolved.capabilities.operator_webui_config,
            "a session minted with operator = true must authenticate with \
             operator capabilities",
        );
    }

    // Escalation-guard tripwire (USER-DECIDED LAW: SSO/multi-user sessions
    // stay non-operator): a session minted with `operator = false` — the
    // shape every OAuth/SSO callback and admin-provisioned-user mint uses —
    // must NEVER resolve to operator capabilities, regardless of how the
    // `SessionRecord` is otherwise constructed. This is the permanent
    // regression pin for the escalation guard the crate docs describe.
    #[tokio::test]
    async fn session_minted_as_non_operator_never_escalates() {
        let store = store();
        let token = store
            .create_session(tenant(), user(), ChronoDuration::hours(1), false)
            .await
            .expect("create");
        let auth = SessionAuthenticator::new(store);
        let resolved = auth
            .authenticate(token.expose_secret())
            .await
            .expect("authenticated");
        assert!(
            !resolved.capabilities.operator_webui_config,
            "a session minted with operator = false must never authenticate \
             with operator capabilities",
        );
    }

    // Fail-closed: a SessionRecord persisted before `operator` existed must
    // deserialize with operator = false, never silently escalate.
    #[test]
    fn pre_fix_session_record_json_without_operator_field_deserializes_non_operator() {
        let json = serde_json::json!({
            "session_id": "11111111-1111-1111-1111-111111111111",
            "tenant_id": "tenant-a",
            "user_id": "alice",
            "created_at": "2024-01-01T00:00:00Z",
            "expires_at": "2024-01-02T00:00:00Z",
        })
        .to_string();
        let record: SessionRecord =
            serde_json::from_str(&json).expect("pre-fix record shape must still deserialize");
        assert!(
            !record.operator,
            "a pre-fix SessionRecord JSON with no `operator` field must default to \
             non-operator",
        );
    }

    // Regression for the session-token-leak review (Medium): the
    // bearer token is the durable store's lookup key, never a field
    // on `SessionRecord`. `Debug` and `Serialize` of a record must
    // therefore never contain the bearer value, so accidental logging
    // (`tracing::debug!(?record, ...)`) or accidental persistence
    // (`serde_json::to_string(&record)`) cannot exfiltrate a live
    // session secret.
    #[tokio::test]
    async fn session_record_debug_and_serialize_do_not_contain_bearer() {
        let store = store();
        let token = store
            .create_session(tenant(), user(), ChronoDuration::hours(1), false)
            .await
            .expect("create");
        let bearer = token.expose_secret().to_string();
        let record = store
            .lookup(&bearer)
            .await
            .expect("lookup")
            .expect("record");

        let debug = format!("{record:?}");
        assert!(
            !debug.contains(&bearer),
            "SessionRecord Debug must not contain the bearer token; got: {debug}",
        );

        let json = serde_json::to_string(&record).expect("serialize");
        assert!(
            !json.contains(&bearer),
            "SessionRecord Serialize must not contain the bearer token; got: {json}",
        );

        // SessionId is present and stable — it is the non-secret
        // audit identifier that consumers may legitimately log.
        assert!(
            json.contains(record.session_id.as_str()),
            "wire shape must carry the non-secret SessionId; got: {json}",
        );
    }
}
