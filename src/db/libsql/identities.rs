//! IdentityStore implementation for LibSqlBackend.
// safety: each async fn uses a single conn.query()/conn.execute() — no multi-op transactions needed

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{fmt_opt_ts, fmt_ts, get_opt_text, get_text, get_ts, opt_text};
use crate::db::libsql::LibSqlBackend;
use crate::db::{DatabaseError, IdentityStore, UserIdentityRecord, UserRecord};

fn row_to_identity(row: &libsql::Row) -> Result<UserIdentityRecord, DatabaseError> {
    let id_str = get_text(row, 0);
    let id: Uuid = id_str
        .parse()
        .map_err(|e| DatabaseError::Serialization(format!("invalid UUID: {e}")))?;
    let raw_str = get_text(row, 8);
    let raw_profile: serde_json::Value =
        serde_json::from_str(&raw_str).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let email_verified_i: i64 = row
        .get::<i64>(5)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    Ok(UserIdentityRecord {
        id,
        user_id: get_text(row, 1),
        provider: get_text(row, 2),
        provider_user_id: get_text(row, 3),
        email: get_opt_text(row, 4),
        email_verified: email_verified_i != 0,
        display_name: get_opt_text(row, 6),
        avatar_url: get_opt_text(row, 7),
        raw_profile,
        created_at: get_ts(row, 9),
        updated_at: get_ts(row, 10),
    })
}

#[async_trait]
impl IdentityStore for LibSqlBackend {
    async fn get_identity_by_provider(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, provider, provider_user_id, email, email_verified, \
                 display_name, avatar_url, raw_profile, created_at, updated_at \
                 FROM user_identities WHERE provider = ?1 AND provider_user_id = ?2",
                params![provider, provider_user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows.next().await {
            Ok(Some(row)) => Ok(Some(row_to_identity(&row)?)),
            Ok(None) => Ok(None),
            Err(e) => Err(DatabaseError::Query(e.to_string())),
        }
    }

    async fn list_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserIdentityRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, provider, provider_user_id, email, email_verified, \
                 display_name, avatar_url, raw_profile, created_at, updated_at \
                 FROM user_identities WHERE user_id = ?1 ORDER BY created_at",
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut result = Vec::new();
        loop {
            match rows.next().await {
                Ok(Some(row)) => result.push(row_to_identity(&row)?),
                Ok(None) => break,
                Err(e) => return Err(DatabaseError::Query(e.to_string())),
            }
        }
        Ok(result)
    }

    async fn create_identity(&self, identity: &UserIdentityRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let raw = serde_json::to_string(&identity.raw_profile)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        conn.execute(
            "INSERT INTO user_identities \
             (id, user_id, provider, provider_user_id, email, email_verified, \
              display_name, avatar_url, raw_profile, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                identity.id.to_string(),
                identity.user_id.as_str(),
                identity.provider.as_str(),
                identity.provider_user_id.as_str(),
                opt_text(identity.email.as_deref()),
                if identity.email_verified { 1i64 } else { 0i64 },
                opt_text(identity.display_name.as_deref()),
                opt_text(identity.avatar_url.as_deref()),
                raw,
                fmt_ts(&identity.created_at),
                fmt_ts(&identity.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn update_identity_profile(
        &self,
        provider: &str,
        provider_user_id: &str,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE user_identities SET \
             display_name = COALESCE(?3, display_name), \
             avatar_url = COALESCE(?4, avatar_url), \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE provider = ?1 AND provider_user_id = ?2",
            params![
                provider,
                provider_user_id,
                opt_text(display_name),
                opt_text(avatar_url),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn find_identity_by_verified_email(
        &self,
        email: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, provider, provider_user_id, email, email_verified, \
                 display_name, avatar_url, raw_profile, created_at, updated_at \
                 FROM user_identities WHERE LOWER(email) = LOWER(?1) AND email_verified = 1 LIMIT 1",
                params![email],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows.next().await {
            Ok(Some(row)) => Ok(Some(row_to_identity(&row)?)),
            Ok(None) => Ok(None),
            Err(e) => Err(DatabaseError::Query(e.to_string())),
        }
    }

    async fn create_user_with_identity(
        &self,
        user: &UserRecord,
        identity: &UserIdentityRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;

        let metadata_str = serde_json::to_string(&user.metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let raw_profile_str = serde_json::to_string(&identity.raw_profile)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute("BEGIN", ()) // safety: this IS a transaction wrapping the multi-step insert below
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // Insert user
        let user_result = conn
            .execute(
                "INSERT INTO users (id, email, display_name, status, role, created_at, \
                 updated_at, last_login_at, created_by, metadata) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    user.id.as_str(),
                    opt_text(user.email.as_deref()),
                    user.display_name.as_str(),
                    user.status.as_str(),
                    user.role.as_str(),
                    fmt_ts(&user.created_at),
                    fmt_ts(&user.updated_at),
                    fmt_opt_ts(&user.last_login_at),
                    opt_text(user.created_by.as_deref()),
                    metadata_str,
                ],
            )
            .await;

        if let Err(e) = user_result {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(DatabaseError::Query(e.to_string()));
        }

        // Insert identity
        let identity_result = conn
            .execute(
                "INSERT INTO user_identities \
                 (id, user_id, provider, provider_user_id, email, email_verified, \
                  display_name, avatar_url, raw_profile, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    identity.id.to_string(),
                    identity.user_id.as_str(),
                    identity.provider.as_str(),
                    identity.provider_user_id.as_str(),
                    opt_text(identity.email.as_deref()),
                    if identity.email_verified { 1i64 } else { 0i64 },
                    opt_text(identity.display_name.as_deref()),
                    opt_text(identity.avatar_url.as_deref()),
                    raw_profile_str,
                    fmt_ts(&identity.created_at),
                    fmt_ts(&identity.updated_at),
                ],
            )
            .await;

        if let Err(e) = identity_result {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(DatabaseError::Query(e.to_string()));
        }

        // Atomically promote to admin if this is the only user in the table.
        // This prevents the TOCTOU race where two concurrent first logins both
        // see an empty users table and both get role=admin.
        let promote_result = conn
            .execute(
                "UPDATE users SET role = 'admin' \
                 WHERE id = ?1 AND (SELECT COUNT(*) FROM users) = 1",
                params![user.id.as_str()],
            )
            .await;

        if let Err(e) = promote_result {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(DatabaseError::Query(e.to_string()));
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, IdentityStore, UserStore};

    async fn test_backend() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_identities.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();
        (backend, dir)
    }

    #[tokio::test]
    async fn test_identity_crud() {
        let (db, _dir) = test_backend().await;
        let now = chrono::Utc::now();

        // Create a user first
        let user = UserRecord {
            id: "user-1".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: "Alice".to_string(),
            status: "active".to_string(),
            role: "member".to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        };
        db.create_user(&user).await.unwrap();

        let identity = UserIdentityRecord {
            id: Uuid::new_v4(),
            user_id: "user-1".to_string(),
            provider: "google".to_string(),
            provider_user_id: "google-sub-123".to_string(),
            email: Some("alice@example.com".to_string()),
            email_verified: true,
            display_name: Some("Alice G".to_string()),
            avatar_url: Some("https://example.com/photo.jpg".to_string()),
            raw_profile: serde_json::json!({"sub": "google-sub-123"}),
            created_at: now,
            updated_at: now,
        };
        db.create_identity(&identity).await.unwrap();

        // Get by provider
        let found = db
            .get_identity_by_provider("google", "google-sub-123")
            .await
            .unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.user_id, "user-1");
        assert_eq!(found.provider, "google");
        assert!(found.email_verified);

        // Not found for wrong provider
        let not_found = db
            .get_identity_by_provider("github", "google-sub-123")
            .await
            .unwrap();
        assert!(not_found.is_none());

        // List for user
        let list = db.list_identities_for_user("user-1").await.unwrap();
        assert_eq!(list.len(), 1);

        // Find by verified email
        let by_email = db
            .find_identity_by_verified_email("alice@example.com")
            .await
            .unwrap();
        assert!(by_email.is_some());
    }

    #[tokio::test]
    async fn test_find_by_verified_email_ignores_unverified() {
        let (db, _dir) = test_backend().await;
        let now = chrono::Utc::now();

        let user = UserRecord {
            id: "user-2".to_string(),
            email: None,
            display_name: "Bob".to_string(),
            status: "active".to_string(),
            role: "member".to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        };
        db.create_user(&user).await.unwrap();

        let identity = UserIdentityRecord {
            id: Uuid::new_v4(),
            user_id: "user-2".to_string(),
            provider: "github".to_string(),
            provider_user_id: "gh-456".to_string(),
            email: Some("bob@example.com".to_string()),
            email_verified: false,
            display_name: None,
            avatar_url: None,
            raw_profile: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        };
        db.create_identity(&identity).await.unwrap();

        let result = db
            .find_identity_by_verified_email("bob@example.com")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_create_user_with_identity() {
        let (db, _dir) = test_backend().await;
        let now = chrono::Utc::now();

        let user = UserRecord {
            id: "user-3".to_string(),
            email: Some("carol@example.com".to_string()),
            display_name: "Carol".to_string(),
            status: "active".to_string(),
            role: "member".to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        };
        let identity = UserIdentityRecord {
            id: Uuid::new_v4(),
            user_id: "user-3".to_string(),
            provider: "google".to_string(),
            provider_user_id: "google-sub-789".to_string(),
            email: Some("carol@example.com".to_string()),
            email_verified: true,
            display_name: Some("Carol".to_string()),
            avatar_url: None,
            raw_profile: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        };
        db.create_user_with_identity(&user, &identity)
            .await
            .unwrap();

        // Verify both records exist
        let found_user = db.get_user("user-3").await.unwrap();
        assert!(found_user.is_some());

        let found_identity = db
            .get_identity_by_provider("google", "google-sub-789")
            .await
            .unwrap();
        assert!(found_identity.is_some());
    }

    /// Regression: an earlier release recorded V15 as "document_versions"
    /// due to a merge-conflict misnumbering. Verify that `run_migrations`
    /// repairs V15 and creates the user_identities table (now V17).
    #[tokio::test]
    async fn test_v15_misnumbered_repair() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_v15_repair.db");
        let backend = LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Simulate the bug: drop user_identities, delete V17 record,
        // and re-record V15 with the wrong name ("document_versions").
        let conn = backend.connect().await.unwrap();
        conn.execute_batch("DROP TABLE IF EXISTS user_identities")
            .await
            .unwrap();
        conn.execute(
            "DELETE FROM _migrations WHERE version = 17",
            libsql::params![],
        )
        .await
        .unwrap();
        conn.execute(
            "UPDATE _migrations SET name = 'document_versions' WHERE version = 15",
            libsql::params![],
        )
        .await
        .unwrap();

        // Confirm the table is gone
        let err = conn
            .query("SELECT 1 FROM user_identities LIMIT 1", ())
            .await;
        assert!(err.is_err(), "user_identities should not exist yet");

        // Re-run migrations — the repair should fix V15, then V17 creates user_identities
        drop(conn);
        backend.run_migrations().await.unwrap();

        // Table should now exist and be queryable
        let conn = backend.connect().await.unwrap();
        let mut rows = conn
            .query("SELECT 1 FROM user_identities LIMIT 1", ())
            .await
            .unwrap();
        // No rows is fine — just verifying the table exists without error
        let _ = rows.next().await;

        // Verify V15 is now recorded as conversation_source_channel
        let mut rows = conn
            .query(
                "SELECT name FROM _migrations WHERE version = 15",
                libsql::params![],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let name: String = row.get(0).unwrap();
        assert_eq!(name, "conversation_source_channel");

        // Verify V17 is recorded as user_identities
        let mut rows = conn
            .query(
                "SELECT name FROM _migrations WHERE version = 17",
                libsql::params![],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let name: String = row.get(0).unwrap();
        assert_eq!(name, "user_identities");
    }
}
