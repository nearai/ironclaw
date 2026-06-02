//! libSQL-backed [`UserDirectory`] for the WebChat v2 SSO login surface.
//!
//! This is the host-supplied production directory the ingress
//! `build_signed_session_login` builder consumes: it maps each
//! authenticated OAuth profile to a stable, distinct `UserId`, so
//! different people logging in become different users (each with their
//! own `owners/<user>` thread subtree). It mirrors the v1 gateway's
//! `resolve_user`: look up by provider identity, else link by a
//! verified email, else create a fresh user.
//!
//! Two tables in a small libSQL file under the reborn home:
//! - `users(id, email, display_name, created_at)`
//! - `user_identities(provider, provider_user_id, user_id, email,
//!   email_verified, created_at)` keyed on `(provider, provider_user_id)`.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use ironclaw_reborn_composition::host_api::UserId;
use ironclaw_reborn_webui_ingress::{
    OAuthProviderName, OAuthUserProfile, UserDirectory, UserDirectoryError,
};
use uuid::Uuid;

/// libSQL-backed user-identity directory.
pub(crate) struct DbUserDirectory {
    db: Arc<libsql::Database>,
}

impl DbUserDirectory {
    /// Open (or create) the user-identity store at `path` and run its
    /// idempotent migrations.
    pub(crate) async fn open(path: &Path) -> anyhow::Result<Self> {
        let db = libsql::Builder::new_local(path)
            .build()
            .await
            .with_context(|| format!("open WebChat user store at {}", path.display()))?;
        let directory = Self { db: Arc::new(db) };
        directory.run_migrations().await?;
        Ok(directory)
    }

    async fn run_migrations(&self) -> anyhow::Result<()> {
        let conn = self.db.connect().context("connect WebChat user store")?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS users (\
                 id TEXT PRIMARY KEY, \
                 email TEXT, \
                 display_name TEXT, \
                 created_at TEXT NOT NULL)",
            (),
        )
        .await
        .context("create users table")?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS user_identities (\
                 provider TEXT NOT NULL, \
                 provider_user_id TEXT NOT NULL, \
                 user_id TEXT NOT NULL, \
                 email TEXT, \
                 email_verified INTEGER NOT NULL, \
                 created_at TEXT NOT NULL, \
                 PRIMARY KEY (provider, provider_user_id))",
            (),
        )
        .await
        .context("create user_identities table")?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_user_identities_verified_email \
                 ON user_identities (email) WHERE email_verified = 1",
            (),
        )
        .await
        .context("create verified-email index")?;
        Ok(())
    }

    async fn connect(&self) -> Result<libsql::Connection, UserDirectoryError> {
        self.db
            .connect()
            .map_err(|err| UserDirectoryError::Backend(err.to_string()))
    }
}

fn backend(err: impl std::fmt::Display) -> UserDirectoryError {
    UserDirectoryError::Backend(err.to_string())
}

fn text_or_null(value: Option<&str>) -> libsql::Value {
    match value {
        Some(text) => libsql::Value::Text(text.to_string()),
        None => libsql::Value::Null,
    }
}

fn to_user_id(raw: String) -> Result<UserId, UserDirectoryError> {
    UserId::new(&raw).map_err(backend)
}

/// First column of the first row of `sql`, as a String, if any.
async fn query_one_string(
    conn: &libsql::Connection,
    sql: &str,
    params: impl libsql::params::IntoParams,
) -> Result<Option<String>, UserDirectoryError> {
    let mut rows = conn.query(sql, params).await.map_err(backend)?;
    match rows.next().await.map_err(backend)? {
        Some(row) => Ok(Some(row.get::<String>(0).map_err(backend)?)),
        None => Ok(None),
    }
}

async fn insert_identity(
    conn: &libsql::Connection,
    provider: &str,
    profile: &OAuthUserProfile,
    user_id: &str,
    created_at: &str,
) -> Result<(), UserDirectoryError> {
    conn.execute(
        "INSERT INTO user_identities \
             (provider, provider_user_id, user_id, email, email_verified, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        libsql::params![
            provider,
            profile.provider_user_id.as_str(),
            user_id,
            text_or_null(profile.email.as_deref()),
            i64::from(profile.email_verified),
            created_at,
        ],
    )
    .await
    .map_err(backend)?;
    Ok(())
}

#[async_trait]
impl UserDirectory for DbUserDirectory {
    async fn resolve(
        &self,
        provider: &OAuthProviderName,
        profile: &OAuthUserProfile,
    ) -> Result<UserId, UserDirectoryError> {
        let conn = self.connect().await?;
        let provider = provider.as_str();

        // 1. Known provider identity → its existing user.
        if let Some(user_id) = query_one_string(
            &conn,
            "SELECT user_id FROM user_identities WHERE provider = ?1 AND provider_user_id = ?2",
            libsql::params![provider, profile.provider_user_id.as_str()],
        )
        .await?
        {
            return to_user_id(user_id);
        }

        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        // 2. Link by a VERIFIED email to an existing user (cross-provider
        //    account linking). Never link on an unverified email — that
        //    would let an attacker claim another user's account by
        //    asserting their address at a provider that does not verify it.
        if profile.email_verified
            && let Some(email) = profile.email.as_deref()
        {
            let email = email.to_ascii_lowercase();
            if let Some(user_id) = query_one_string(
                &conn,
                "SELECT user_id FROM user_identities \
                     WHERE email_verified = 1 AND lower(email) = ?1 LIMIT 1",
                libsql::params![email],
            )
            .await?
            {
                insert_identity(&conn, provider, profile, &user_id, &now).await?;
                return to_user_id(user_id);
            }
        }

        // 3. New user.
        let new_user_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO users (id, email, display_name, created_at) VALUES (?1, ?2, ?3, ?4)",
            libsql::params![
                new_user_id.as_str(),
                text_or_null(profile.email.as_deref()),
                text_or_null(profile.display_name.as_deref()),
                now.as_str(),
            ],
        )
        .await
        .map_err(backend)?;
        insert_identity(&conn, provider, profile, &new_user_id, &now).await?;
        to_user_id(new_user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn google() -> OAuthProviderName {
        OAuthProviderName::new("google").expect("provider")
    }
    fn github() -> OAuthProviderName {
        OAuthProviderName::new("github").expect("provider")
    }
    fn profile(sub: &str, email: Option<&str>, verified: bool) -> OAuthUserProfile {
        OAuthUserProfile {
            provider_user_id: sub.to_string(),
            email: email.map(str::to_string),
            email_verified: verified,
            display_name: None,
        }
    }

    async fn dir() -> DbUserDirectory {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Leak the tempdir so the libsql file outlives the test body.
        let path = tmp.keep().join("users.db");
        DbUserDirectory::open(&path).await.expect("open")
    }

    #[tokio::test]
    async fn same_provider_identity_is_stable_across_logins() {
        let dir = dir().await;
        let p = google();
        let first = dir
            .resolve(&p, &profile("g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        let second = dir
            .resolve(&p, &profile("g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(first.as_str(), second.as_str());
    }

    #[tokio::test]
    async fn distinct_identities_get_distinct_users() {
        let dir = dir().await;
        let p = google();
        let a = dir
            .resolve(&p, &profile("g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        let b = dir
            .resolve(&p, &profile("g-2", Some("b@x.com"), true))
            .await
            .expect("resolve");
        assert_ne!(
            a.as_str(),
            b.as_str(),
            "different people must be different users"
        );
    }

    #[tokio::test]
    async fn verified_email_links_across_providers() {
        let dir = dir().await;
        let via_google = dir
            .resolve(&google(), &profile("g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let via_github = dir
            .resolve(&github(), &profile("gh-9", Some("same@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            via_google.as_str(),
            via_github.as_str(),
            "a verified shared email links the two provider identities to one user"
        );
    }

    #[tokio::test]
    async fn unverified_email_does_not_link() {
        let dir = dir().await;
        let verified = dir
            .resolve(&google(), &profile("g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        // A different provider asserting the SAME email but UNVERIFIED must
        // not hijack the verified user's account.
        let unverified = dir
            .resolve(&github(), &profile("gh-9", Some("same@x.com"), false))
            .await
            .expect("resolve");
        assert_ne!(
            verified.as_str(),
            unverified.as_str(),
            "an unverified email must never link to a verified account"
        );
    }
}
