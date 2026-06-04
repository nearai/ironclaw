//! libSQL implementation of [`RebornIdentityResolver`](crate::RebornIdentityResolver).
//!
//! Two tables:
//! - `users(id, email, display_name, status, role, created_at, updated_at)`
//! - `external_identities(tenant_id, surface_kind, provider_kind,
//!   provider_instance_id, external_subject_id, user_id, email,
//!   email_verified, created_at)` keyed on the first five columns, with a
//!   partial index over `(tenant_id, lower(email))` for verified-email
//!   cross-provider linking.
//!
//! [`resolve_or_create`](RebornLibSqlIdentityStore::resolve_or_create) runs
//! the lookup → link → create sequence inside one `BEGIN IMMEDIATE`
//! transaction, so concurrent first-contacts for the same identity or the
//! same verified email cannot split into two users or lose the link.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use ironclaw_host_api::{TenantId, UserId};
use uuid::Uuid;

use crate::{
    ExternalIdentityKey, ExternalIdentityRecord, RebornIdentityError, RebornIdentityResolver,
    ResolveExternalIdentity, SurfaceKind, UserRecord,
};

/// libSQL-backed canonical identity store.
pub struct RebornLibSqlIdentityStore {
    db: Arc<libsql::Database>,
}

impl RebornLibSqlIdentityStore {
    /// Open the store on an existing libSQL substrate handle and run its
    /// idempotent migrations.
    pub async fn open(db: Arc<libsql::Database>) -> Result<Self, RebornIdentityError> {
        let store = Self { db };
        store.run_migrations().await?;
        Ok(store)
    }

    /// A connection with a busy timeout set. This store shares the reborn
    /// substrate DB file with the runtime's other handles, so a contended
    /// write must WAIT for the lock rather than fail immediately with
    /// `SQLITE_BUSY`. The timeout is per-connection, so it is set on every
    /// connection here.
    async fn conn(&self) -> Result<libsql::Connection, RebornIdentityError> {
        let conn = self.db.connect().map_err(backend)?;
        // `PRAGMA busy_timeout = N` returns the new value as a row, so it
        // goes through `query` (not `execute`, which rejects row-returning
        // statements). The returned `Rows` is dropped unread.
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(backend)?;
        Ok(conn)
    }

    async fn run_migrations(&self) -> Result<(), RebornIdentityError> {
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
             CREATE TABLE IF NOT EXISTS external_identities (\
                 tenant_id TEXT NOT NULL, \
                 surface_kind TEXT NOT NULL, \
                 provider_kind TEXT NOT NULL, \
                 provider_instance_id TEXT NOT NULL, \
                 external_subject_id TEXT NOT NULL, \
                 user_id TEXT NOT NULL, \
                 email TEXT, \
                 email_verified INTEGER NOT NULL, \
                 created_at TEXT NOT NULL, \
                 PRIMARY KEY (tenant_id, surface_kind, provider_kind, \
                     provider_instance_id, external_subject_id)); \
             CREATE INDEX IF NOT EXISTS idx_external_identities_verified_email \
                 ON external_identities (tenant_id, lower(email)) WHERE email_verified = 1;",
        )
        .await
        .map_err(backend)?;
        Ok(())
    }

    /// Fold legacy WebUI OAuth identities into the canonical store.
    ///
    /// The pre-#4381 WebUI store wrote `user_identities(provider,
    /// provider_user_id, user_id, email, email_verified, created_at)` into
    /// this same substrate DB, keyed only by `(provider, provider_user_id)`
    /// — single-tenant, OAuth-only. This copies those rows into
    /// `external_identities` under `tenant_id` as `oauth`-surface identities
    /// (no provider instance; the legacy subject is the external subject),
    /// reproducing the exact key the live OAuth path resolves with, so
    /// existing SSO users keep their `UserId` across upgrade instead of
    /// being re-minted into orphaned accounts. The shared `users` rows
    /// already live in the same table, so no user copy is needed.
    ///
    /// Idempotent: `INSERT OR IGNORE` skips keys already present, and the
    /// step is a no-op when the legacy table is absent (fresh installs).
    /// Run once at startup, before accepting logins. Returns the number of
    /// legacy identities folded in.
    pub async fn migrate_legacy_webui_identities(
        &self,
        tenant_id: &TenantId,
    ) -> Result<u64, RebornIdentityError> {
        let conn = self.conn().await?;
        if !legacy_user_identities_table_exists(&conn).await? {
            return Ok(0);
        }
        // Bind the surface via `SurfaceKind::Oauth.as_str()` rather than a
        // literal so the migrated key cannot drift from the live path's key.
        conn.execute(
            "INSERT OR IGNORE INTO external_identities \
                 (tenant_id, surface_kind, provider_kind, provider_instance_id, \
                  external_subject_id, user_id, email, email_verified, created_at) \
             SELECT ?1, ?2, provider, '', provider_user_id, user_id, email, \
                    email_verified, created_at \
             FROM user_identities",
            libsql::params![tenant_id.as_str(), SurfaceKind::Oauth.as_str()],
        )
        .await
        .map_err(backend)
    }
}

#[async_trait]
impl RebornIdentityResolver for RebornLibSqlIdentityStore {
    /// Resolve an external identity to a stable `UserId`, atomically:
    /// 1. Known `(tenant, surface, provider, instance, subject)` key → its
    ///    existing user.
    /// 2. Else, a VERIFIED email matching an existing verified identity
    ///    *in the same tenant* → link this identity to that user.
    /// 3. Else, a brand-new user + identity.
    async fn resolve_or_create(
        &self,
        identity: ResolveExternalIdentity<'_>,
    ) -> Result<UserId, RebornIdentityError> {
        let tenant = identity.tenant_id.as_str();
        let surface = identity.surface_kind.as_str();
        // No installation (browser OAuth) collapses to "" so the composite
        // primary key stays total — a NULL key part would make every row
        // distinct and defeat the lookup.
        let instance = identity.provider_instance_id.unwrap_or("");

        let conn = self.conn().await?;

        // Fast path: a returning external identity (the common case)
        // resolves with a read-only query and never takes the IMMEDIATE
        // write lock, so logins for already-provisioned users don't
        // serialize behind a write transaction.
        if let Some(user_id) = select_identity_user(
            &conn,
            tenant,
            surface,
            identity.provider_kind,
            instance,
            identity.external_subject_id,
        )
        .await?
        {
            return to_user_id(user_id);
        }

        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(backend)?;

        // 1. Re-check the external identity key under the write lock: a
        //    concurrent first-login for the same key may have inserted it
        //    between the read above and acquiring the lock, so the create
        //    path below must not race to a second user.
        if let Some(user_id) = select_identity_user(
            &tx,
            tenant,
            surface,
            identity.provider_kind,
            instance,
            identity.external_subject_id,
        )
        .await?
        {
            tx.commit().await.map_err(backend)?;
            return to_user_id(user_id);
        }

        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        // 2. Link by a VERIFIED email to an existing user in the SAME
        //    tenant. Never link on an unverified email — that would let an
        //    attacker claim another user's account by asserting their
        //    address at a provider that does not verify it. Tenant-scoped
        //    so two tenants sharing an email address stay separate users.
        if identity.email_verified
            && let Some(email) = identity.email
        {
            let email_lc = email.to_ascii_lowercase();
            if let Some(user_id) = query_one_string(
                &tx,
                "SELECT user_id FROM external_identities \
                     WHERE tenant_id = ?1 AND email_verified = 1 AND lower(email) = ?2 LIMIT 1",
                libsql::params![tenant, email_lc],
            )
            .await?
            {
                let user_id = to_user_id(user_id)?;
                insert_identity(&tx, &identity_record(&identity, instance, &user_id, &now)).await?;
                tx.commit().await.map_err(backend)?;
                return Ok(user_id);
            }
        }

        // 3. New user.
        let new_user_id = to_user_id(Uuid::new_v4().to_string())?;
        insert_user(
            &tx,
            &UserRecord {
                id: new_user_id.clone(),
                email: identity.email.map(str::to_string),
                display_name: identity.display_name.map(str::to_string),
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .await?;
        insert_identity(
            &tx,
            &identity_record(&identity, instance, &new_user_id, &now),
        )
        .await?;
        tx.commit().await.map_err(backend)?;
        Ok(new_user_id)
    }

    /// Link-only lookup — never creates. Returns the user already bound to
    /// this external identity key, or `None`.
    async fn lookup(
        &self,
        key: ExternalIdentityKey<'_>,
    ) -> Result<Option<UserId>, RebornIdentityError> {
        let instance = key.provider_instance_id.unwrap_or("");
        let conn = self.conn().await?;
        match select_identity_user(
            &conn,
            key.tenant_id.as_str(),
            key.surface_kind.as_str(),
            key.provider_kind,
            instance,
            key.external_subject_id,
        )
        .await?
        {
            Some(user_id) => Ok(Some(to_user_id(user_id)?)),
            None => Ok(None),
        }
    }

    /// Link an external identity to an already-existing user. No user row
    /// is created (link-only); re-binding the same key re-points it at
    /// `user_id`. Channel actors carry no email, so the row stores none.
    async fn bind(
        &self,
        key: ExternalIdentityKey<'_>,
        user_id: &UserId,
    ) -> Result<(), RebornIdentityError> {
        let instance = key.provider_instance_id.unwrap_or("");
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let conn = self.conn().await?;
        conn.execute(
            "INSERT INTO external_identities \
                 (tenant_id, surface_kind, provider_kind, provider_instance_id, \
                  external_subject_id, user_id, email, email_verified, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, ?7) \
             ON CONFLICT (tenant_id, surface_kind, provider_kind, provider_instance_id, \
                 external_subject_id) DO UPDATE SET user_id = excluded.user_id",
            libsql::params![
                key.tenant_id.as_str(),
                key.surface_kind.as_str(),
                key.provider_kind,
                instance,
                key.external_subject_id,
                user_id.as_str(),
                now.as_str()
            ],
        )
        .await
        .map_err(backend)?;
        Ok(())
    }
}

fn identity_record(
    identity: &ResolveExternalIdentity<'_>,
    instance: &str,
    user_id: &UserId,
    created_at: &str,
) -> ExternalIdentityRecord {
    ExternalIdentityRecord {
        tenant_id: identity.tenant_id.clone(),
        surface_kind: identity.surface_kind,
        provider_kind: identity.provider_kind.to_string(),
        provider_instance_id: instance.to_string(),
        external_subject_id: identity.external_subject_id.to_string(),
        user_id: user_id.clone(),
        email: identity.email.map(str::to_string),
        email_verified: identity.email_verified,
        created_at: created_at.to_string(),
    }
}

fn backend(err: impl std::fmt::Display) -> RebornIdentityError {
    RebornIdentityError::Backend(err.to_string())
}

fn text_or_null(value: Option<&str>) -> libsql::Value {
    match value {
        Some(text) => libsql::Value::Text(text.to_string()),
        None => libsql::Value::Null,
    }
}

fn to_user_id(raw: String) -> Result<UserId, RebornIdentityError> {
    UserId::new(&raw).map_err(|err| RebornIdentityError::InvalidUserId(err.to_string()))
}

/// First column of the first row, as a `String`, if any.
/// Whether the legacy pre-#4381 WebUI `user_identities` table is present in
/// this substrate DB. Used to make legacy migration a no-op on fresh DBs.
async fn legacy_user_identities_table_exists(
    conn: &libsql::Connection,
) -> Result<bool, RebornIdentityError> {
    Ok(query_one_string(
        conn,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'user_identities'",
        (),
    )
    .await?
    .is_some())
}

/// Look up the user bound to an external-identity key. Shared by the
/// `resolve_or_create` fast path, its in-transaction re-check, and
/// `lookup`, so the keyed SELECT lives in exactly one place. Accepts a
/// `&Connection` or (via deref) a `&Transaction`.
async fn select_identity_user(
    conn: &libsql::Connection,
    tenant: &str,
    surface: &str,
    provider_kind: &str,
    instance: &str,
    subject: &str,
) -> Result<Option<String>, RebornIdentityError> {
    query_one_string(
        conn,
        "SELECT user_id FROM external_identities \
             WHERE tenant_id = ?1 AND surface_kind = ?2 AND provider_kind = ?3 \
                 AND provider_instance_id = ?4 AND external_subject_id = ?5",
        libsql::params![tenant, surface, provider_kind, instance, subject],
    )
    .await
}

async fn query_one_string(
    conn: &libsql::Connection,
    sql: &str,
    params: impl libsql::params::IntoParams,
) -> Result<Option<String>, RebornIdentityError> {
    let mut rows = conn.query(sql, params).await.map_err(backend)?;
    match rows.next().await.map_err(backend)? {
        Some(row) => Ok(Some(row.get::<String>(0).map_err(backend)?)),
        None => Ok(None),
    }
}

async fn insert_user(
    conn: &libsql::Connection,
    user: &UserRecord,
) -> Result<(), RebornIdentityError> {
    // `status` / `role` are intentionally omitted; the `users` table fills
    // them from its column DEFAULTs (`active` / `member`). See UserRecord.
    conn.execute(
        "INSERT INTO users \
             (id, email, display_name, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![
            user.id.as_str(),
            text_or_null(user.email.as_deref()),
            text_or_null(user.display_name.as_deref()),
            user.created_at.as_str(),
            user.updated_at.as_str(),
        ],
    )
    .await
    .map_err(backend)?;
    Ok(())
}

async fn insert_identity(
    conn: &libsql::Connection,
    identity: &ExternalIdentityRecord,
) -> Result<(), RebornIdentityError> {
    conn.execute(
        "INSERT INTO external_identities \
             (tenant_id, surface_kind, provider_kind, provider_instance_id, \
              external_subject_id, user_id, email, email_verified, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        libsql::params![
            identity.tenant_id.as_str(),
            identity.surface_kind.as_str(),
            identity.provider_kind.as_str(),
            identity.provider_instance_id.as_str(),
            identity.external_subject_id.as_str(),
            identity.user_id.as_str(),
            text_or_null(identity.email.as_deref()),
            i64::from(identity.email_verified),
            identity.created_at.as_str(),
        ],
    )
    .await
    .map_err(backend)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SurfaceKind;
    use ironclaw_host_api::TenantId;

    async fn store() -> RebornLibSqlIdentityStore {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Leak the tempdir so the libSQL file outlives the test body.
        let path = tmp.keep().join("reborn-local-dev.db");
        let db = Arc::new(
            libsql::Builder::new_local(&path)
                .build()
                .await
                .expect("open libsql"),
        );
        RebornLibSqlIdentityStore::open(db)
            .await
            .expect("open store")
    }

    fn tenant(id: &str) -> TenantId {
        TenantId::new(id).expect("tenant")
    }

    fn oauth<'a>(
        tenant: &'a TenantId,
        provider: &'a str,
        sub: &'a str,
        email: Option<&'a str>,
        verified: bool,
    ) -> ResolveExternalIdentity<'a> {
        ResolveExternalIdentity {
            tenant_id: tenant,
            surface_kind: SurfaceKind::Oauth,
            provider_kind: provider,
            provider_instance_id: None,
            external_subject_id: sub,
            email,
            email_verified: verified,
            display_name: None,
        }
    }

    fn channel_actor<'a>(
        tenant: &'a TenantId,
        provider: &'a str,
        instance: &'a str,
        actor: &'a str,
    ) -> ResolveExternalIdentity<'a> {
        ResolveExternalIdentity {
            tenant_id: tenant,
            surface_kind: SurfaceKind::ChannelActor,
            provider_kind: provider,
            provider_instance_id: Some(instance),
            external_subject_id: actor,
            email: None,
            email_verified: false,
            display_name: None,
        }
    }

    #[tokio::test]
    async fn same_identity_is_stable_across_logins() {
        let store = store().await;
        let t = tenant("t");
        let first = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        let second = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(first.as_str(), second.as_str());
    }

    #[tokio::test]
    async fn distinct_identities_get_distinct_users() {
        let store = store().await;
        let t = tenant("t");
        let a = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("a@x.com"), true))
            .await
            .expect("resolve");
        let b = store
            .resolve_or_create(oauth(&t, "google", "g-2", Some("b@x.com"), true))
            .await
            .expect("resolve");
        assert_ne!(
            a.as_str(),
            b.as_str(),
            "different people are different users"
        );
    }

    #[tokio::test]
    async fn verified_email_links_across_oauth_providers() {
        let store = store().await;
        let t = tenant("t");
        let via_google = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let via_github = store
            .resolve_or_create(oauth(&t, "github", "gh-9", Some("same@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            via_google.as_str(),
            via_github.as_str(),
            "a verified shared email links both provider identities to one user"
        );
    }

    #[tokio::test]
    async fn verified_email_link_is_case_insensitive() {
        // Two providers assert the SAME verified email in DIFFERENT casing.
        // Linking lowercases both the stored and the queried address, so
        // they must still collapse to one user — regression guard for the
        // cross-provider linking rule under mixed-case provider claims.
        let store = store().await;
        let t = tenant("t");
        let via_google = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("Alice@Example.COM"), true))
            .await
            .expect("resolve");
        let via_github = store
            .resolve_or_create(oauth(&t, "github", "gh-9", Some("alice@example.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            via_google.as_str(),
            via_github.as_str(),
            "verified-email linking must be case-insensitive across providers"
        );
    }

    #[tokio::test]
    async fn migrate_legacy_webui_identities_preserves_user_id_across_upgrade() {
        // A pre-#4381 deployment has legacy `user_identities` rows in the
        // shared substrate DB. After upgrade, migration must fold them into
        // `external_identities` so the user's NEXT login resolves to their
        // EXISTING UserId instead of minting a fresh, orphaned account.
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.keep().join("reborn-local-dev.db");
        let db = Arc::new(
            libsql::Builder::new_local(&path)
                .build()
                .await
                .expect("open libsql"),
        );

        // Seed the legacy WebUI identity exactly as the old store wrote it.
        let seed = db.connect().expect("connect");
        seed.execute_batch(
            "CREATE TABLE user_identities (\
                 provider TEXT NOT NULL, provider_user_id TEXT NOT NULL, \
                 user_id TEXT NOT NULL, email TEXT, email_verified INTEGER NOT NULL, \
                 created_at TEXT NOT NULL, \
                 PRIMARY KEY (provider, provider_user_id));",
        )
        .await
        .expect("seed legacy schema");
        seed.execute(
            "INSERT INTO user_identities \
                 (provider, provider_user_id, user_id, email, email_verified, created_at) \
                 VALUES ('google', 'g-legacy', 'legacy-user-7', 'legacy@x.com', 1, \
                     '2026-01-01T00:00:00Z')",
            (),
        )
        .await
        .expect("seed legacy identity");

        let store = RebornLibSqlIdentityStore::open(db)
            .await
            .expect("open store");
        let t = tenant("t");
        let migrated = store
            .migrate_legacy_webui_identities(&t)
            .await
            .expect("migration runs");
        assert_eq!(migrated, 1, "exactly one legacy identity is folded in");

        let resolved = store
            .resolve_or_create(oauth(&t, "google", "g-legacy", Some("legacy@x.com"), true))
            .await
            .expect("resolve");
        assert_eq!(
            resolved.as_str(),
            "legacy-user-7",
            "a returning legacy SSO user keeps their original UserId after upgrade"
        );

        let again = store
            .migrate_legacy_webui_identities(&t)
            .await
            .expect("re-run");
        assert_eq!(again, 0, "re-running the migration folds nothing new");
    }

    #[tokio::test]
    async fn migrate_legacy_webui_identities_is_noop_without_legacy_table() {
        // Fresh installs have no `user_identities` table; migration must be
        // a clean no-op rather than erroring on the missing table.
        let store = store().await;
        let migrated = store
            .migrate_legacy_webui_identities(&tenant("t"))
            .await
            .expect("no-op migration succeeds");
        assert_eq!(migrated, 0);
    }

    #[tokio::test]
    async fn unverified_email_does_not_link() {
        let store = store().await;
        let t = tenant("t");
        let verified = store
            .resolve_or_create(oauth(&t, "google", "g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let unverified = store
            .resolve_or_create(oauth(&t, "github", "gh-9", Some("same@x.com"), false))
            .await
            .expect("resolve");
        assert_ne!(
            verified.as_str(),
            unverified.as_str(),
            "an unverified email must never link to a verified account"
        );
    }

    #[tokio::test]
    async fn different_tenant_does_not_collide_on_same_subject() {
        let store = store().await;
        let (a, b) = (tenant("tenant-a"), tenant("tenant-b"));
        let in_a = store
            .resolve_or_create(oauth(&a, "google", "g-1", Some("u@x.com"), true))
            .await
            .expect("resolve");
        let in_b = store
            .resolve_or_create(oauth(&b, "google", "g-1", Some("u@x.com"), true))
            .await
            .expect("resolve");
        assert_ne!(
            in_a.as_str(),
            in_b.as_str(),
            "the same provider subject in two tenants must be two users"
        );
    }

    #[tokio::test]
    async fn verified_email_link_is_tenant_scoped() {
        let store = store().await;
        let (a, b) = (tenant("tenant-a"), tenant("tenant-b"));
        let in_a = store
            .resolve_or_create(oauth(&a, "google", "g-1", Some("same@x.com"), true))
            .await
            .expect("resolve");
        let in_b = store
            .resolve_or_create(oauth(&b, "github", "gh-9", Some("same@x.com"), true))
            .await
            .expect("resolve");
        assert_ne!(
            in_a.as_str(),
            in_b.as_str(),
            "a shared verified email must not link accounts across tenants"
        );
    }

    #[tokio::test]
    async fn different_provider_instance_does_not_collide() {
        let store = store().await;
        let t = tenant("t");
        let i1 = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-7"))
            .await
            .expect("resolve");
        let i2 = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-2", "actor-7"))
            .await
            .expect("resolve");
        assert_ne!(
            i1.as_str(),
            i2.as_str(),
            "the same actor id under two installations must be two users"
        );
    }

    #[tokio::test]
    async fn channel_actor_without_email_is_stable_and_distinct() {
        let store = store().await;
        let t = tenant("t");
        let a1 = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-1"))
            .await
            .expect("resolve");
        let a1_again = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-1"))
            .await
            .expect("resolve");
        let a2 = store
            .resolve_or_create(channel_actor(&t, "telegram", "inst-1", "actor-2"))
            .await
            .expect("resolve");
        assert_eq!(a1.as_str(), a1_again.as_str(), "same actor is stable");
        assert_ne!(
            a1.as_str(),
            a2.as_str(),
            "distinct actors are distinct users"
        );
    }

    #[tokio::test]
    async fn concurrent_first_logins_for_one_email_resolve_to_one_user() {
        // Two providers asserting the SAME verified email at the same time
        // must converge on ONE user, not split into two. The IMMEDIATE
        // transaction serializes the second behind the first so it sees the
        // freshly-linked verified email.
        let store = Arc::new(store().await);
        let (a, b) = (store.clone(), store.clone());
        let (ra, rb) = tokio::join!(
            tokio::spawn(async move {
                let t = tenant("t");
                a.resolve_or_create(oauth(&t, "google", "g-1", Some("dup@x.com"), true))
                    .await
            }),
            tokio::spawn(async move {
                let t = tenant("t");
                b.resolve_or_create(oauth(&t, "github", "gh-1", Some("dup@x.com"), true))
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

    fn channel_key<'a>(
        tenant: &'a TenantId,
        provider: &'a str,
        subject: &'a str,
    ) -> ExternalIdentityKey<'a> {
        ExternalIdentityKey {
            tenant_id: tenant,
            surface_kind: SurfaceKind::ChannelActor,
            provider_kind: provider,
            provider_instance_id: None,
            external_subject_id: subject,
        }
    }

    #[tokio::test]
    async fn lookup_unbound_actor_returns_none() {
        let store = store().await;
        let t = tenant("t");
        assert!(
            store
                .lookup(channel_key(&t, "slack", "U-unbound"))
                .await
                .expect("lookup")
                .is_none(),
            "an unbound channel actor must fail closed (None), never auto-provision"
        );
    }

    #[tokio::test]
    async fn bind_then_lookup_returns_bound_user() {
        let store = store().await;
        let t = tenant("t");
        let user = UserId::new("user-7").expect("user");
        store
            .bind(channel_key(&t, "slack", "U-1"), &user)
            .await
            .expect("bind");
        let resolved = store
            .lookup(channel_key(&t, "slack", "U-1"))
            .await
            .expect("lookup");
        assert_eq!(resolved.as_ref().map(|u| u.as_str()), Some("user-7"));
    }

    #[tokio::test]
    async fn rebind_repoints_to_new_user() {
        let store = store().await;
        let t = tenant("t");
        store
            .bind(
                channel_key(&t, "slack", "U-1"),
                &UserId::new("user-a").unwrap(),
            )
            .await
            .expect("bind a");
        store
            .bind(
                channel_key(&t, "slack", "U-1"),
                &UserId::new("user-b").unwrap(),
            )
            .await
            .expect("rebind b");
        let resolved = store
            .lookup(channel_key(&t, "slack", "U-1"))
            .await
            .expect("lookup");
        assert_eq!(
            resolved.as_ref().map(|u| u.as_str()),
            Some("user-b"),
            "re-binding re-points the actor to the new user"
        );
    }

    #[tokio::test]
    async fn bind_is_scoped_per_key_and_tenant_and_surface() {
        let store = store().await;
        let t = tenant("t");
        store
            .bind(
                channel_key(&t, "slack", "U-1"),
                &UserId::new("user-a").unwrap(),
            )
            .await
            .expect("bind");
        // Different actor id → different (unbound) key.
        assert!(
            store
                .lookup(channel_key(&t, "slack", "U-2"))
                .await
                .expect("lookup")
                .is_none()
        );
        // Same actor id in another tenant → unbound.
        let other_tenant = tenant("other");
        assert!(
            store
                .lookup(channel_key(&other_tenant, "slack", "U-1"))
                .await
                .expect("lookup")
                .is_none()
        );
        // A channel-actor binding must not satisfy an oauth-surface lookup
        // even with identical provider/subject — surface_kind separates them.
        let oauth_same = ExternalIdentityKey {
            tenant_id: &t,
            surface_kind: SurfaceKind::Oauth,
            provider_kind: "slack",
            provider_instance_id: None,
            external_subject_id: "U-1",
        };
        assert!(store.lookup(oauth_same).await.expect("lookup").is_none());
    }
}
