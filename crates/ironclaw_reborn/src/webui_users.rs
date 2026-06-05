//! libSQL-backed user + OAuth-identity store for the WebChat v2 SSO
//! login surface — the reborn-owned persistence layer the host's
//! `UserDirectory` delegates to.
//!
//! This mirrors the [`RebornLibSqlSecretStore`](crate::secrets) pattern:
//! it takes the shared reborn libSQL substrate handle (the same
//! `reborn-local-dev.db` the runtime opens — *not* a separate identity
//! database), runs its own idempotent migrations, and fails closed. It
//! is deliberately free of any web / ingress dependency: callers hand it
//! a plain [`ResolveIdentity`], never the ingress `OAuthUserProfile`, so
//! the storage layer never sees raw provider claims.
//!
//! Two tables:
//! - `users(id, email, display_name, status, role, created_at, updated_at)`
//! - `user_identities(provider, provider_user_id, user_id, email,
//!   email_verified, created_at)` keyed on `(provider, provider_user_id)`,
//!   with a partial index over verified emails for cross-provider linking.
//!
//! [`resolve_or_create`](RebornLibSqlUserStore::resolve_or_create) runs
//! the lookup → link → create sequence inside a single `BEGIN IMMEDIATE`
//! transaction, so two concurrent first-logins for the same identity or
//! the same verified email cannot split into two users or lose the link
//! (the write lock is taken at `BEGIN`, serializing the callbacks).

use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use ironclaw_host_api::UserId;
use libsql::params_from_iter;
use thiserror::Error;
use uuid::Uuid;

/// Failure modes of the libSQL user store.
#[derive(Debug, Error)]
pub enum RebornUserStoreError {
    /// The libSQL backend (connect / migrate / query / commit) failed.
    #[error("reborn user store backend failure: {0}")]
    Backend(String),
    /// A persisted user id failed `UserId` validation on read-back — a
    /// backend inconsistency, surfaced rather than silently dropped.
    #[error("persisted user id is invalid: {0}")]
    InvalidUserId(String),
}

/// Provider-normalized identity handed to
/// [`RebornLibSqlUserStore::resolve_or_create`]. Plain borrowed fields so
/// this storage crate stays independent of the ingress profile type.
pub struct ResolveIdentity<'a> {
    /// Provider name (e.g. `google`, `github`).
    pub provider: &'a str,
    /// Stable per-provider subject id (Google `sub`, GitHub numeric id).
    pub provider_user_id: &'a str,
    /// Email claimed by the provider, if any.
    pub email: Option<&'a str>,
    /// Whether the provider asserts the email is verified.
    pub email_verified: bool,
    /// Optional display name.
    pub display_name: Option<&'a str>,
}

/// libSQL-backed user-identity repository.
pub struct RebornLibSqlUserStore {
    db: Arc<libsql::Database>,
}

impl RebornLibSqlUserStore {
    /// Open the store on an existing libSQL substrate handle and run its
    /// idempotent migrations.
    pub async fn open(db: Arc<libsql::Database>) -> Result<Self, RebornUserStoreError> {
        let store = Self { db };
        store.run_migrations().await?;
        Ok(store)
    }

    /// A connection with a busy timeout set. This store shares the reborn
    /// substrate DB file with the runtime's filesystem store (a second
    /// handle on the same SQLite file), so a contended write must WAIT for
    /// the lock rather than fail immediately with `SQLITE_BUSY`. The
    /// timeout is per-connection, so it is set on every connection here.
    async fn conn(&self) -> Result<libsql::Connection, RebornUserStoreError> {
        let conn = self.db.connect().map_err(backend)?;
        // `PRAGMA busy_timeout = N` returns the new value as a row, so it
        // goes through `query` (not `execute`, which rejects row-returning
        // statements). The returned `Rows` is dropped unread.
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(backend)?;
        Ok(conn)
    }

    async fn run_migrations(&self) -> Result<(), RebornUserStoreError> {
        let conn = self.conn().await?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (\
                 id TEXT PRIMARY KEY, \
                 email TEXT, \
                 display_name TEXT, \
                 status TEXT NOT NULL DEFAULT 'active', \
                 role TEXT NOT NULL DEFAULT 'member', \
                 created_at TEXT NOT NULL, \
                 updated_at TEXT NOT NULL); \
             CREATE TABLE IF NOT EXISTS user_identities (\
                 provider TEXT NOT NULL, \
                 provider_user_id TEXT NOT NULL, \
                 user_id TEXT NOT NULL, \
                 email TEXT, \
                 email_verified INTEGER NOT NULL, \
                 created_at TEXT NOT NULL, \
                 PRIMARY KEY (provider, provider_user_id)); \
             DROP INDEX IF EXISTS idx_user_identities_verified_email; \
             CREATE INDEX IF NOT EXISTS idx_user_identities_verified_email_lower \
                 ON user_identities (lower(email)) WHERE email_verified = 1;",
        )
        .await
        .map_err(backend)?;
        Ok(())
    }

    /// Resolve a provider identity to a stable `UserId`, creating or
    /// linking as needed, atomically:
    /// 1. Known `(provider, provider_user_id)` → its existing user.
    /// 2. Else, a VERIFIED email matching an existing verified identity →
    ///    link this identity to that user (cross-provider account link).
    /// 3. Else, a brand-new user + identity.
    ///
    /// The whole sequence runs in one `BEGIN IMMEDIATE` transaction so
    /// concurrent first-logins serialize instead of racing into duplicate
    /// users or a lost link.
    pub async fn resolve_or_create(
        &self,
        identity: ResolveIdentity<'_>,
    ) -> Result<UserId, RebornUserStoreError> {
        let conn = self.conn().await?;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(backend)?;

        // 1. Known provider identity → its existing user.
        if let Some(user_id) = query_one_string(
            &tx,
            "SELECT user_id FROM user_identities WHERE provider = ?1 AND provider_user_id = ?2",
            libsql::params![identity.provider, identity.provider_user_id],
        )
        .await?
        {
            tx.commit().await.map_err(backend)?;
            return to_user_id(user_id);
        }

        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        // 2. Link by a VERIFIED email to an existing user. Never link on
        //    an unverified email — that would let an attacker claim
        //    another user's account by asserting their address at a
        //    provider that does not verify it.
        if identity.email_verified
            && let Some(email) = identity.email
        {
            let email_lc = email.to_ascii_lowercase();
            if let Some(user_id) = query_one_string(
                &tx,
                "SELECT user_id FROM user_identities \
                     WHERE email_verified = 1 AND lower(email) = ?1 LIMIT 1",
                libsql::params![email_lc],
            )
            .await?
            {
                insert_identity(&tx, &identity, &user_id, &now).await?;
                tx.commit().await.map_err(backend)?;
                return to_user_id(user_id);
            }
        }

        // 3. New user.
        let new_user_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO users \
                 (id, email, display_name, status, role, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 'active', 'member', ?4, ?4)",
            libsql::params![
                new_user_id.as_str(),
                text_or_null(identity.email),
                text_or_null(identity.display_name),
                now.as_str(),
            ],
        )
        .await
        .map_err(backend)?;
        insert_identity(&tx, &identity, &new_user_id, &now).await?;
        tx.commit().await.map_err(backend)?;
        to_user_id(new_user_id)
    }

    /// List active users whose persisted email domain is currently admitted.
    ///
    /// Used by local-dev SSO startup to reconcile trigger-fire access for
    /// users that were created in an earlier process before the trigger poller
    /// was enabled. Domain matching is case-insensitive and an empty allowlist
    /// returns no users.
    pub async fn list_active_users_by_allowed_email_domains(
        &self,
        allowed_email_domains: &[String],
    ) -> Result<Vec<UserId>, RebornUserStoreError> {
        let allowed_patterns: Vec<String> = allowed_email_domains
            .iter()
            .map(|domain| domain.trim().to_ascii_lowercase())
            .filter(|domain| !domain.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|domain| format!("%@{}", escape_like_pattern(&domain)))
            .collect();
        if allowed_patterns.is_empty() {
            return Ok(Vec::new());
        }

        let domain_predicates =
            std::iter::repeat_n("lower(email) LIKE ? ESCAPE '\\'", allowed_patterns.len())
                .collect::<Vec<_>>()
                .join(" OR ");
        let sql = format!(
            "SELECT id \
             FROM users \
             WHERE status = 'active' \
               AND email IS NOT NULL \
               AND ({domain_predicates})"
        );
        let conn = self.conn().await?;
        let mut rows = conn
            .query(&sql, params_from_iter(allowed_patterns))
            .await
            .map_err(backend)?;
        let mut users = Vec::new();
        while let Some(row) = rows.next().await.map_err(backend)? {
            let user_id = row.get::<String>(0).map_err(backend)?;
            users.push(to_user_id(user_id)?);
        }
        Ok(users)
    }
}

fn backend(err: impl std::fmt::Display) -> RebornUserStoreError {
    RebornUserStoreError::Backend(err.to_string())
}

fn text_or_null(value: Option<&str>) -> libsql::Value {
    match value {
        Some(text) => libsql::Value::Text(text.to_string()),
        None => libsql::Value::Null,
    }
}

fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn to_user_id(raw: String) -> Result<UserId, RebornUserStoreError> {
    UserId::new(&raw).map_err(|err| RebornUserStoreError::InvalidUserId(err.to_string()))
}

/// First column of the first row, as a `String`, if any.
async fn query_one_string(
    conn: &libsql::Connection,
    sql: &str,
    params: impl libsql::params::IntoParams,
) -> Result<Option<String>, RebornUserStoreError> {
    let mut rows = conn.query(sql, params).await.map_err(backend)?;
    match rows.next().await.map_err(backend)? {
        Some(row) => Ok(Some(row.get::<String>(0).map_err(backend)?)),
        None => Ok(None),
    }
}

async fn insert_identity(
    conn: &libsql::Connection,
    identity: &ResolveIdentity<'_>,
    user_id: &str,
    created_at: &str,
) -> Result<(), RebornUserStoreError> {
    conn.execute(
        "INSERT INTO user_identities \
             (provider, provider_user_id, user_id, email, email_verified, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        libsql::params![
            identity.provider,
            identity.provider_user_id,
            user_id,
            text_or_null(identity.email),
            i64::from(identity.email_verified),
            created_at,
        ],
    )
    .await
    .map_err(backend)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> RebornLibSqlUserStore {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Leak the tempdir so the libSQL file outlives the test body.
        let path = tmp.keep().join("reborn-local-dev.db");
        let db = Arc::new(
            libsql::Builder::new_local(&path)
                .build()
                .await
                .expect("open libsql"),
        );
        RebornLibSqlUserStore::open(db).await.expect("open store")
    }

    fn identity<'a>(
        provider: &'a str,
        sub: &'a str,
        email: Option<&'a str>,
        verified: bool,
    ) -> ResolveIdentity<'a> {
        ResolveIdentity {
            provider,
            provider_user_id: sub,
            email,
            email_verified: verified,
            display_name: None,
        }
    }

    #[tokio::test]
    async fn same_identity_is_stable_across_logins() {
        let store = store().await;
        let first = store
            .resolve_or_create(identity("google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        let second = store
            .resolve_or_create(identity("google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(first.as_str(), second.as_str());
    }

    #[tokio::test]
    async fn distinct_identities_get_distinct_users() {
        let store = store().await;
        let a = store
            .resolve_or_create(identity("google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        let b = store
            .resolve_or_create(identity("google", "g-2", Some("b@x.com"), true))
            .await
            .expect("resolve");
        assert_ne!(
            a.as_str(),
            b.as_str(),
            "different people are different users"
        );
    }

    #[tokio::test]
    async fn verified_email_links_across_providers() {
        let store = store().await;
        let via_google = store
            .resolve_or_create(identity("google", "g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let via_github = store
            .resolve_or_create(identity("github", "gh-9", Some("same@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            via_google.as_str(),
            via_github.as_str(),
            "a verified shared email links both provider identities to one user"
        );
    }

    #[tokio::test]
    async fn unverified_email_does_not_link() {
        let store = store().await;
        let verified = store
            .resolve_or_create(identity("google", "g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let unverified = store
            .resolve_or_create(identity("github", "gh-9", Some("same@x.com"), false))
            .await
            .expect("resolve");
        assert_ne!(
            verified.as_str(),
            unverified.as_str(),
            "an unverified email must never link to a verified account"
        );
    }

    #[tokio::test]
    async fn concurrent_first_logins_for_one_email_resolve_to_one_user() {
        // Two providers asserting the SAME verified email at the same time
        // must converge on ONE user, not split into two. The IMMEDIATE
        // transaction serializes the second behind the first so it sees
        // the freshly-linked verified email.
        let store = Arc::new(store().await);
        let a = store.clone();
        let b = store.clone();
        let (ra, rb) = tokio::join!(
            tokio::spawn(async move {
                a.resolve_or_create(identity("google", "g-1", Some("dup@x.com"), true))
                    .await
            }),
            tokio::spawn(async move {
                b.resolve_or_create(identity("github", "gh-1", Some("dup@x.com"), true))
                    .await
            }),
        );
        let user_a = ra.expect("join").expect("resolve");
        let user_b = rb.expect("join").expect("resolve");
        assert_eq!(
            user_a.as_str(),
            user_b.as_str(),
            "concurrent first-logins for one verified email must share a user"
        );

        let conn = store.conn().await.expect("conn");
        let count = query_one_string(&conn, "SELECT CAST(COUNT(*) AS TEXT) FROM users", ())
            .await
            .expect("count")
            .expect("row");
        assert_eq!(count, "1", "exactly one user row must exist");
    }

    #[tokio::test]
    async fn list_active_users_by_allowed_email_domains_filters_current_admission() {
        let store = store().await;
        let allowed = store
            .resolve_or_create(identity("google", "g-1", Some("a@example.com"), true))
            .await
            .expect("resolve allowed");
        let allowed_case = store
            .resolve_or_create(identity("google", "g-2", Some("b@Example.COM"), true))
            .await
            .expect("resolve mixed-case allowed");
        let inactive = store
            .resolve_or_create(identity("google", "g-3", Some("c@example.com"), true))
            .await
            .expect("resolve inactive");
        store
            .resolve_or_create(identity("google", "g-4", Some("d@other.test"), true))
            .await
            .expect("resolve disallowed");

        let conn = store.conn().await.expect("conn");
        conn.execute(
            "UPDATE users SET status = 'inactive' WHERE id = ?1",
            libsql::params![inactive.as_str()],
        )
        .await
        .expect("deactivate user");

        let users = store
            .list_active_users_by_allowed_email_domains(&["EXAMPLE.com".to_string()])
            .await
            .expect("list active users");
        let user_ids: BTreeSet<&str> = users.iter().map(UserId::as_str).collect();

        assert!(user_ids.contains(allowed.as_str()));
        assert!(user_ids.contains(allowed_case.as_str()));
        assert!(!user_ids.contains(inactive.as_str()));
        assert_eq!(user_ids.len(), 2);
    }

    #[tokio::test]
    async fn list_active_users_by_allowed_email_domains_empty_allowlist_fails_closed() {
        let store = store().await;
        store
            .resolve_or_create(identity("google", "g-1", Some("a@example.com"), true))
            .await
            .expect("resolve");

        let users = store
            .list_active_users_by_allowed_email_domains(&[])
            .await
            .expect("list active users");

        assert!(users.is_empty());
    }

    #[tokio::test]
    async fn list_active_users_by_allowed_email_domains_treats_like_wildcards_literally() {
        let store = store().await;
        store
            .resolve_or_create(identity("google", "g-1", Some("a@example.com"), true))
            .await
            .expect("resolve normal domain");
        let literal_percent = store
            .resolve_or_create(identity("google", "g-2", Some("b@%example.com"), true))
            .await
            .expect("resolve literal percent domain");

        let users = store
            .list_active_users_by_allowed_email_domains(&["%example.com".to_string()])
            .await
            .expect("list active users");

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].as_str(), literal_percent.as_str());
    }

    #[tokio::test]
    async fn list_active_users_by_allowed_email_domains_treats_underscore_literally() {
        let store = store().await;
        store
            .resolve_or_create(identity("google", "g-1", Some("a@axexample.com"), true))
            .await
            .expect("resolve wildcard-looking domain");
        let literal_underscore = store
            .resolve_or_create(identity("google", "g-2", Some("b@a_example.com"), true))
            .await
            .expect("resolve literal underscore domain");

        let users = store
            .list_active_users_by_allowed_email_domains(&["a_example.com".to_string()])
            .await
            .expect("list active users");

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].as_str(), literal_underscore.as_str());
    }

    #[tokio::test]
    async fn list_active_users_by_allowed_email_domains_treats_backslash_literally() {
        let store = store().await;
        store
            .resolve_or_create(identity("google", "g-1", Some("a@example.com"), true))
            .await
            .expect("resolve normal domain");
        let literal_backslash = store
            .resolve_or_create(identity("google", "g-2", Some("b@exa\\mple.com"), true))
            .await
            .expect("resolve literal backslash domain");

        let users = store
            .list_active_users_by_allowed_email_domains(&["exa\\mple.com".to_string()])
            .await
            .expect("list active users");

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].as_str(), literal_backslash.as_str());
    }
}
