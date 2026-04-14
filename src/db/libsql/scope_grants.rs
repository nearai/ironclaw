//! ScopeGrantStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;

use super::{get_i64, get_opt_text, get_opt_ts, get_text, get_ts};
use crate::db::libsql::LibSqlBackend;
use crate::db::{DatabaseError, ScopeGrantRecord, ScopeGrantStore};

fn row_to_scope_grant(row: &libsql::Row) -> Result<ScopeGrantRecord, DatabaseError> {
    Ok(ScopeGrantRecord {
        user_id: get_text(row, 0),
        scope: get_text(row, 1),
        writable: get_i64(row, 2) != 0,
        granted_by: get_opt_text(row, 3),
        created_at: get_ts(row, 4),
        expires_at: get_opt_ts(row, 5),
    })
}

#[async_trait]
impl ScopeGrantStore for LibSqlBackend {
    async fn list_scope_grants(
        &self,
        user_id: &str,
    ) -> Result<Vec<ScopeGrantRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT user_id, scope, writable, granted_by, created_at, expires_at \
                 FROM scope_grants WHERE user_id = ?1 ORDER BY created_at",
                params![user_id],
            )
            .await?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(row_to_scope_grant(&row)?);
        }
        Ok(results)
    }

    async fn set_scope_grant(
        &self,
        user_id: &str,
        scope: &str,
        writable: bool,
        granted_by: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let expires_str = expires_at.map(|dt| dt.to_rfc3339());
        conn.execute(
            "INSERT INTO scope_grants (user_id, scope, writable, granted_by, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (user_id, scope) DO UPDATE SET \
                 writable = excluded.writable, \
                 granted_by = excluded.granted_by, \
                 expires_at = excluded.expires_at",
            params![
                user_id,
                scope,
                if writable { 1i64 } else { 0i64 },
                granted_by,
                expires_str
            ],
        )
        .await?;
        Ok(())
    }

    async fn revoke_scope_grant(
        &self,
        user_id: &str,
        scope: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let n = conn
            .execute(
                "DELETE FROM scope_grants WHERE user_id = ?1 AND scope = ?2",
                params![user_id, scope],
            )
            .await?;
        Ok(n > 0)
    }

    async fn revoke_scope_grant_by_granter(
        &self,
        user_id: &str,
        scope: &str,
        granted_by: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let n = conn
            .execute(
                "DELETE FROM scope_grants WHERE user_id = ?1 AND scope = ?2 AND granted_by = ?3",
                params![user_id, scope, granted_by],
            )
            .await?;
        Ok(n > 0)
    }

    async fn get_scope_grant(
        &self,
        user_id: &str,
        scope: &str,
    ) -> Result<Option<ScopeGrantRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT user_id, scope, writable, granted_by, created_at, expires_at \
                 FROM scope_grants WHERE user_id = ?1 AND scope = ?2",
                params![user_id, scope],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(Some(row_to_scope_grant(&row)?)),
            None => Ok(None),
        }
    }

    async fn has_writable_grant(
        &self,
        user_id: &str,
        scope: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM scope_grants WHERE user_id = ?1 AND scope = ?2 AND writable = 1",
                params![user_id, scope],
            )
            .await?;
        Ok(rows.next().await?.is_some())
    }

    async fn list_scope_grants_for_scope(
        &self,
        scope: &str,
    ) -> Result<Vec<ScopeGrantRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT user_id, scope, writable, granted_by, created_at, expires_at \
                 FROM scope_grants WHERE scope = ?1 ORDER BY created_at",
                params![scope],
            )
            .await?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(row_to_scope_grant(&row)?);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{Database, ScopeGrantStore};

    #[tokio::test]
    async fn scope_grant_crud() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_scope_grants.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Initially empty
        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert!(grants.is_empty());

        // Set a read-only grant (no expiration)
        backend
            .set_scope_grant("alice", "shared", false, Some("admin"), None)
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].scope, "shared");
        assert!(!grants[0].writable);
        assert_eq!(grants[0].granted_by.as_deref(), Some("admin"));
        assert!(grants[0].expires_at.is_none());

        // Upgrade to writable via upsert
        backend
            .set_scope_grant("alice", "shared", true, Some("admin"), None)
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1);
        assert!(grants[0].writable);

        // Add another grant
        backend
            .set_scope_grant("alice", "team", true, None, None)
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 2);

        // List by scope
        let scope_grants = backend
            .list_scope_grants_for_scope("shared")
            .await
            .unwrap();
        assert_eq!(scope_grants.len(), 1);
        assert_eq!(scope_grants[0].user_id, "alice");

        // Get single grant
        let grant = backend.get_scope_grant("alice", "shared").await.unwrap();
        assert!(grant.is_some());
        assert!(grant.unwrap().writable);

        let no_grant = backend.get_scope_grant("alice", "missing").await.unwrap();
        assert!(no_grant.is_none());

        // Revoke
        let revoked = backend.revoke_scope_grant("alice", "shared").await.unwrap();
        assert!(revoked);

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].scope, "team");

        // Revoke non-existent returns false
        let revoked = backend.revoke_scope_grant("alice", "shared").await.unwrap();
        assert!(!revoked);
    }

    #[tokio::test]
    async fn scope_grant_expires_at() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_scope_grants_exp.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        backend
            .set_scope_grant("alice", "shared", false, Some("admin"), Some(future))
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1);
        assert!(grants[0].expires_at.is_some());

        // Upsert to clear expiration
        backend
            .set_scope_grant("alice", "shared", false, Some("admin"), None)
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert!(grants[0].expires_at.is_none());
    }

    #[tokio::test]
    async fn scope_grant_revoke_by_granter() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_scope_grants_granter.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Grant by "bob"
        backend
            .set_scope_grant("alice", "shared", false, Some("bob"), None)
            .await
            .unwrap();

        // Try to revoke as "charlie" -- should fail (wrong granter)
        let revoked = backend
            .revoke_scope_grant_by_granter("alice", "shared", "charlie")
            .await
            .unwrap();
        assert!(!revoked);

        // Revoke as "bob" -- should succeed
        let revoked = backend
            .revoke_scope_grant_by_granter("alice", "shared", "bob")
            .await
            .unwrap();
        assert!(revoked);

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert!(grants.is_empty());
    }

    #[tokio::test]
    async fn expired_grants_still_stored() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_expired_stored.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Set a grant with past expiry
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        backend
            .set_scope_grant("alice", "shared", false, Some("admin"), Some(past))
            .await
            .unwrap();

        // DB stores everything; filtering is auth-layer responsibility
        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1, "expired grant should still be in DB");
        assert_eq!(grants[0].scope, "shared");
        assert!(grants[0].expires_at.is_some());
    }

    #[tokio::test]
    async fn get_scope_grant_returns_correct_fields() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_get_fields.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        let future = chrono::Utc::now() + chrono::Duration::hours(2);
        backend
            .set_scope_grant("alice", "shared", true, Some("bob"), Some(future))
            .await
            .unwrap();

        let grant = backend
            .get_scope_grant("alice", "shared")
            .await
            .unwrap()
            .expect("grant should exist");
        assert_eq!(grant.user_id, "alice");
        assert_eq!(grant.scope, "shared");
        assert!(grant.writable);
        assert_eq!(grant.granted_by.as_deref(), Some("bob"));
        assert!(grant.expires_at.is_some());
        // Verify the stored expiry is within a reasonable range of what we set
        let stored = grant.expires_at.unwrap();
        let diff = (stored - future).num_seconds().abs();
        assert!(diff < 2, "expires_at should round-trip accurately");
    }

    #[tokio::test]
    async fn has_writable_grant_with_expired_grant() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_writable_expired.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Set a writable grant with past expiry
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        backend
            .set_scope_grant("alice", "shared", true, Some("admin"), Some(past))
            .await
            .unwrap();

        // DB doesn't filter by expiry; auth layer does
        let has = backend.has_writable_grant("alice", "shared").await.unwrap();
        assert!(has, "DB should return true regardless of expiry");
    }

    #[tokio::test]
    async fn revoke_by_granter_with_null_granted_by() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_null_granter.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Set a grant with no granter (granted_by = None)
        backend
            .set_scope_grant("alice", "shared", false, None, None)
            .await
            .unwrap();

        // Attempting to revoke as anyone should fail (NULL != "anyone")
        let revoked = backend
            .revoke_scope_grant_by_granter("alice", "shared", "anyone")
            .await
            .unwrap();
        assert!(!revoked, "cannot revoke NULL-granted grant by granter name");

        // Grant should still exist
        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1);
    }

    #[tokio::test]
    async fn multiple_grants_for_same_user() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_multi_grants.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        backend
            .set_scope_grant("alice", "scope1", false, Some("admin"), None)
            .await
            .unwrap();
        backend
            .set_scope_grant("alice", "scope2", true, Some("admin"), None)
            .await
            .unwrap();
        backend
            .set_scope_grant("alice", "scope3", false, Some("bob"), None)
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 3);
        let scopes: Vec<&str> = grants.iter().map(|g| g.scope.as_str()).collect();
        assert!(scopes.contains(&"scope1"));
        assert!(scopes.contains(&"scope2"));
        assert!(scopes.contains(&"scope3"));
    }

    #[tokio::test]
    async fn upsert_changes_writable_and_expires_at() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_upsert_both.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Start with read-only, no expiry
        backend
            .set_scope_grant("alice", "shared", false, Some("admin"), None)
            .await
            .unwrap();

        let grant = backend
            .get_scope_grant("alice", "shared")
            .await
            .unwrap()
            .unwrap();
        assert!(!grant.writable);
        assert!(grant.expires_at.is_none());

        // Upsert to writable with expiry
        let future = chrono::Utc::now() + chrono::Duration::hours(24);
        backend
            .set_scope_grant("alice", "shared", true, Some("admin"), Some(future))
            .await
            .unwrap();

        let grant = backend
            .get_scope_grant("alice", "shared")
            .await
            .unwrap()
            .unwrap();
        assert!(grant.writable, "upsert should change writable to true");
        assert!(
            grant.expires_at.is_some(),
            "upsert should set expires_at"
        );

        // Verify only one grant exists (upsert, not duplicate)
        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1);
    }

    #[tokio::test]
    async fn list_scope_grants_for_scope_multiple_grantees() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_multi_grantees.db");
        let backend = super::LibSqlBackend::new_local(&db_path).await.unwrap();
        backend.run_migrations().await.unwrap();

        // Grant alice and bob access to "shared"
        backend
            .set_scope_grant("alice", "shared", false, Some("admin"), None)
            .await
            .unwrap();
        backend
            .set_scope_grant("bob", "shared", true, Some("admin"), None)
            .await
            .unwrap();

        let grants = backend
            .list_scope_grants_for_scope("shared")
            .await
            .unwrap();
        assert_eq!(grants.len(), 2);
        let user_ids: Vec<&str> = grants.iter().map(|g| g.user_id.as_str()).collect();
        assert!(user_ids.contains(&"alice"));
        assert!(user_ids.contains(&"bob"));

        // Verify writable flags are correct
        let alice_grant = grants.iter().find(|g| g.user_id == "alice").unwrap();
        assert!(!alice_grant.writable);
        let bob_grant = grants.iter().find(|g| g.user_id == "bob").unwrap();
        assert!(bob_grant.writable);
    }
}
