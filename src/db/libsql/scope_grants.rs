//! ScopeGrantStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;

use super::{get_i64, get_opt_text, get_text, get_ts};
use crate::db::libsql::LibSqlBackend;
use crate::db::{DatabaseError, ScopeGrantRecord, ScopeGrantStore};

fn row_to_scope_grant(row: &libsql::Row) -> Result<ScopeGrantRecord, DatabaseError> {
    Ok(ScopeGrantRecord {
        user_id: get_text(row, 0),
        scope: get_text(row, 1),
        writable: get_i64(row, 2) != 0,
        granted_by: get_opt_text(row, 3),
        created_at: get_ts(row, 4),
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
                "SELECT user_id, scope, writable, granted_by, created_at \
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
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO scope_grants (user_id, scope, writable, granted_by) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (user_id, scope) DO UPDATE SET \
                 writable = excluded.writable, \
                 granted_by = excluded.granted_by",
            params![
                user_id,
                scope,
                if writable { 1i64 } else { 0i64 },
                granted_by
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

    async fn list_scope_grants_for_scope(
        &self,
        scope: &str,
    ) -> Result<Vec<ScopeGrantRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT user_id, scope, writable, granted_by, created_at \
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

        // Set a read-only grant
        backend
            .set_scope_grant("alice", "shared", false, Some("admin"))
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].scope, "shared");
        assert!(!grants[0].writable);
        assert_eq!(grants[0].granted_by.as_deref(), Some("admin"));

        // Upgrade to writable via upsert
        backend
            .set_scope_grant("alice", "shared", true, Some("admin"))
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 1);
        assert!(grants[0].writable);

        // Add another grant
        backend
            .set_scope_grant("alice", "team", true, None)
            .await
            .unwrap();

        let grants = backend.list_scope_grants("alice").await.unwrap();
        assert_eq!(grants.len(), 2);

        // List by scope
        let scope_grants = backend.list_scope_grants_for_scope("shared").await.unwrap();
        assert_eq!(scope_grants.len(), 1);
        assert_eq!(scope_grants[0].user_id, "alice");

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
}
