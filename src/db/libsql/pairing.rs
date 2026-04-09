//! ChannelPairingStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;

use super::{LibSqlBackend, fmt_ts, get_ts, opt_text};
use crate::db::{ChannelPairingStore, DatabaseError, PairingRequestRecord};
use crate::ownership::{Identity, OwnerId, UserRole};

#[async_trait]
impl ChannelPairingStore for LibSqlBackend {
    async fn resolve_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<Option<Identity>, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT ci.owner_id, u.role
                 FROM channel_identities ci
                 JOIN users u ON u.id = ci.owner_id
                 WHERE ci.channel = ?1 AND ci.external_id = ?2
                   AND u.status = 'active'
                 LIMIT 1",
                params![channel, external_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => {
                let owner_id: String = row
                    .get(0)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                let role_str: String = row
                    .get(1)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                let role = if role_str.eq_ignore_ascii_case("admin") {
                    UserRole::Admin
                } else {
                    UserRole::Member
                };
                Ok(Some(Identity::new(OwnerId::from(owner_id), role)))
            }
            None => Ok(None),
        }
    }

    async fn read_allow_from(&self, channel: &str) -> Result<Vec<String>, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT ci.external_id
                 FROM channel_identities ci
                 JOIN users u ON u.id = ci.owner_id
                 WHERE ci.channel = ?1
                   AND u.status = 'active'
                 ORDER BY ci.external_id ASC",
                params![channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            result.push(
                row.get(0)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
            );
        }

        Ok(result)
    }

    async fn upsert_pairing_request(
        &self,
        channel: &str,
        external_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<PairingRequestRecord, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let conn = self.connect().await?;

        // safety: BEGIN IMMEDIATE acquires a write lock upfront, preventing concurrent upserts
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            let now = chrono::Utc::now();
            let now_str = fmt_ts(&now);
            conn.execute(
                "UPDATE pairing_requests
                 SET expires_at = ?3
                 WHERE channel = ?1 AND external_id = ?2
                   AND approved_at IS NULL
                   AND expires_at > ?3",
                params![channel.as_str(), external_id, now_str.as_str()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            let now = chrono::Utc::now();
            let expires_at = now + chrono::Duration::minutes(15);
            let now_str = fmt_ts(&now);
            let expires_str = fmt_ts(&expires_at);
            let meta_str = meta.map(|v| v.to_string());

            // Retry loop: regenerate code on UNIQUE constraint violation (code collision)
            for attempt in 0..3 {
                let id = uuid::Uuid::new_v4().to_string();
                let code = crate::db::generate_pairing_code();
                match conn
                    .execute(
                        "INSERT INTO pairing_requests (id, channel, external_id, code, meta, created_at, expires_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            id.as_str(),
                            channel.as_str(),
                            external_id,
                            code.as_str(),
                            opt_text(meta_str.as_deref()),
                            now_str.as_str(),
                            expires_str.as_str()
                        ],
                    )
                    .await
                {
                    Ok(_) => {
                        return Ok(PairingRequestRecord {
                            id: uuid::Uuid::parse_str(&id)
                                .map_err(|e| DatabaseError::Query(e.to_string()))?,
                            channel: channel.to_string(),
                            external_id: external_id.to_string(),
                            code,
                            created: true,
                            created_at: now,
                            expires_at,
                        });
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        if attempt < 2 && msg.contains("UNIQUE constraint failed") {
                            continue;
                        }
                        return Err(DatabaseError::Query(msg));
                    }
                }
            }

            Err(DatabaseError::Query(
                "failed to generate unique pairing code after 3 attempts".to_string(),
            ))
        }
        .await;

        match result {
            Ok(record) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(record)
            }
            Err(e) => {
                // Best-effort rollback — if rollback fails, log but return original error
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }

    async fn approve_pairing(
        &self,
        channel: &str,
        code: &str,
        owner_id: &str,
    ) -> Result<(), DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let conn = self.connect().await?;

        // safety: BEGIN IMMEDIATE acquires a write lock upfront, preventing concurrent approvals
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // All operations in a closure so we can rollback on any error
        let result = async {
            let mut rows = conn
                .query(
                    "SELECT id, channel, external_id FROM pairing_requests
                     WHERE UPPER(code) = UPPER(?1)
                       AND channel = ?2
                       AND approved_at IS NULL
                       AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                     LIMIT 1",
                    libsql::params![code, channel],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

            let row = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(format!("approve_pairing SELECT: {e}")))?
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "pairing_request".into(),
                    id: code.to_string(),
                })?;

            let req_id: String = row
                .get(0)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let req_channel: String = row
                .get(1)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let external_id: String = row
                .get(2)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let now_str = fmt_ts(&chrono::Utc::now());

            conn.execute(
                "UPDATE pairing_requests SET owner_id = ?1, approved_at = ?2 WHERE id = ?3",
                libsql::params![owner_id, now_str.as_str(), req_id.as_str()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            let identity_id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO channel_identities (id, owner_id, channel, external_id)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (channel, external_id) DO UPDATE SET owner_id = ?2",
                libsql::params![
                    identity_id.as_str(),
                    owner_id,
                    req_channel.as_str(),
                    external_id.as_str()
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            Ok::<(), DatabaseError>(())
        }
        .await;

        match result {
            Ok(()) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(())
            }
            Err(e) => {
                // Best-effort rollback — if rollback fails, log but return original error
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }

    async fn list_pending_pairings(
        &self,
        channel: &str,
    ) -> Result<Vec<PairingRequestRecord>, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, channel, external_id, code, created_at, expires_at
                 FROM pairing_requests
                 WHERE channel = ?1 AND approved_at IS NULL
                   AND expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 ORDER BY created_at ASC",
                params![channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let id_str: String = row
                .get(0)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            result.push(PairingRequestRecord {
                id: uuid::Uuid::parse_str(&id_str)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
                channel: row
                    .get(1)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
                external_id: row
                    .get(2)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
                code: row
                    .get(3)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
                created: false,
                created_at: get_ts(&row, 4),
                expires_at: get_ts(&row, 5),
            });
        }
        Ok(result)
    }

    async fn remove_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<(), DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let conn = self.connect().await?;
        conn.execute(
            "DELETE FROM channel_identities WHERE channel = ?1 AND external_id = ?2",
            params![channel, external_id],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::libsql::LibSqlBackend;
    use crate::db::{ChannelPairingStore, Database, UserRecord, UserStore};

    /// Create a file-backed in-memory-like DB.
    ///
    /// `new_memory()` uses an in-memory SQLite connection which does NOT share
    /// schema across separate `connect()` calls in libsql. We use a temp file so
    /// all connections within the same test share the migrated schema.
    async fn setup_db() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("pairing_test.db");
        let db = LibSqlBackend::new_local(&db_path).await.unwrap();
        db.run_migrations().await.unwrap();
        (db, dir)
    }

    async fn setup_db_with_user(user_id: &str) -> (LibSqlBackend, tempfile::TempDir) {
        let (db, dir) = setup_db().await;
        db.get_or_create_user(UserRecord {
            id: user_id.to_string(),
            role: "member".to_string(),
            display_name: user_id.to_string(),
            status: "active".to_string(),
            email: None,
            last_login_at: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
            max_agents: None,
            max_tokens: None,
        })
        .await
        .unwrap();
        (db, dir)
    }

    #[tokio::test]
    async fn test_resolve_identity_unknown_sender() {
        let (db, _dir) = setup_db().await;
        let result = db
            .resolve_channel_identity("telegram", "unknown123")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_upsert_and_approve_pairing() {
        let (db, _dir) = setup_db_with_user("alice").await;

        let req = db
            .upsert_pairing_request("telegram", "tg-alice-123", None)
            .await
            .unwrap();
        assert_eq!(req.channel, "telegram");
        assert_eq!(req.external_id, "tg-alice-123");
        assert_eq!(req.code.len(), 8);
        assert!(req.created, "first upsert should set created = true");

        // Before approval: still unknown
        assert!(
            db.resolve_channel_identity("telegram", "tg-alice-123")
                .await
                .unwrap()
                .is_none()
        );

        // Approve
        db.approve_pairing("telegram", &req.code, "alice")
            .await
            .unwrap();

        // After approval: resolves to alice
        let identity = db
            .resolve_channel_identity("telegram", "tg-alice-123")
            .await
            .unwrap();
        assert!(identity.is_some());
        assert_eq!(identity.unwrap().owner_id.as_str(), "alice");
    }

    #[tokio::test]
    async fn test_upsert_rotates_pending_request_code() {
        let (db, _dir) = setup_db_with_user("alice").await;

        let r1 = db
            .upsert_pairing_request("telegram", "user123", None)
            .await
            .unwrap();
        let r2 = db
            .upsert_pairing_request("telegram", "user123", None)
            .await
            .unwrap();
        assert!(r1.created, "first upsert should set created = true");
        assert!(r2.created, "second upsert should create a fresh code");
        assert_ne!(
            r1.code, r2.code,
            "retrying pairing should rotate to a fresh code"
        );

        let err = db.approve_pairing("telegram", &r1.code, "alice").await;
        assert!(
            err.is_err(),
            "retired pairing code should no longer approve"
        );
        assert_eq!(
            db.list_pending_pairings("telegram").await.unwrap().len(),
            1,
            "only the latest pending request should remain active"
        );
    }

    #[tokio::test]
    async fn test_upsert_pairing_request_preserves_null_meta() {
        let (db, _dir) = setup_db().await;
        let req = db
            .upsert_pairing_request("telegram", "user_null_meta", None)
            .await
            .unwrap();

        let conn = db.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT meta FROM pairing_requests WHERE id = ?1",
                libsql::params![req.id.to_string()],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("pairing request row");
        let meta: Option<String> = row.get(0).ok();
        assert!(
            meta.is_none(),
            "None meta should be stored as SQL NULL, got {meta:?}"
        );
    }

    #[tokio::test]
    async fn test_approve_invalid_code_returns_error() {
        let (db, _dir) = setup_db().await;
        let err = db.approve_pairing("telegram", "BADCODE1", "alice").await;
        assert!(err.is_err(), "Invalid code should return error");
    }

    #[tokio::test]
    async fn test_remove_channel_identity() {
        let (db, _dir) = setup_db_with_user("alice").await;
        let req = db
            .upsert_pairing_request("telegram", "tg-remove-test", None)
            .await
            .unwrap();
        db.approve_pairing("telegram", &req.code, "alice")
            .await
            .unwrap();
        assert!(
            db.resolve_channel_identity("telegram", "tg-remove-test")
                .await
                .unwrap()
                .is_some()
        );

        db.remove_channel_identity("telegram", "tg-remove-test")
            .await
            .unwrap();
        assert!(
            db.resolve_channel_identity("telegram", "tg-remove-test")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_list_pending_pairings() {
        let (db, _dir) = setup_db().await;

        let pending_before = db.list_pending_pairings("telegram").await.unwrap();
        assert!(pending_before.is_empty());

        db.upsert_pairing_request("telegram", "user_a", None)
            .await
            .unwrap();
        db.upsert_pairing_request("telegram", "user_b", None)
            .await
            .unwrap();

        let pending = db.list_pending_pairings("telegram").await.unwrap();
        assert_eq!(pending.len(), 2);
    }

    #[tokio::test]
    async fn test_approve_pairing_case_insensitive() {
        let (db, _dir) = setup_db_with_user("alice").await;
        let req = db
            .upsert_pairing_request("telegram", "user_ci", None)
            .await
            .unwrap();

        // Approve with lowercase version of the code
        let lowercase_code = req.code.to_lowercase();
        db.approve_pairing("telegram", &lowercase_code, "alice")
            .await
            .unwrap();

        // Identity should resolve
        let identity = db
            .resolve_channel_identity("telegram", "user_ci")
            .await
            .unwrap();
        assert!(identity.is_some(), "Lowercase code should work");
        assert_eq!(identity.unwrap().owner_id.as_str(), "alice");
    }

    #[tokio::test]
    async fn test_approve_expired_code_rejected() {
        let (db, _dir) = setup_db_with_user("alice").await;

        // Insert an expired pairing request directly
        {
            let conn = db.connect().await.unwrap();
            conn.execute(
                "INSERT INTO pairing_requests (id, channel, external_id, code, created_at, expires_at)
                 VALUES (?1, 'telegram', 'user_expired', 'EXPIRED1', ?2, ?3)",
                libsql::params![
                    uuid::Uuid::new_v4().to_string().as_str(),
                    crate::db::libsql::fmt_ts(&chrono::Utc::now()).as_str(),
                    crate::db::libsql::fmt_ts(
                        &(chrono::Utc::now() - chrono::Duration::hours(1))
                    )
                    .as_str(),
                ],
            )
            .await
            .unwrap();
        }

        let err = db.approve_pairing("telegram", "EXPIRED1", "alice").await;
        assert!(err.is_err(), "Expired code should be rejected");
    }

    #[tokio::test]
    async fn test_approve_same_code_twice_rejected() {
        let (db, _dir) = setup_db_with_user("alice").await;
        let req = db
            .upsert_pairing_request("telegram", "user_double", None)
            .await
            .unwrap();

        // First approval succeeds
        db.approve_pairing("telegram", &req.code, "alice")
            .await
            .unwrap();

        // Second approval with same code fails
        let err = db.approve_pairing("telegram", &req.code, "alice").await;
        assert!(
            err.is_err(),
            "Approving an already-approved code should fail"
        );
    }

    #[tokio::test]
    async fn test_pairing_channel_names_are_canonicalized() {
        let (db, _dir) = setup_db_with_user("alice").await;

        let req = db
            .upsert_pairing_request("TeleGram", "user_case", None)
            .await
            .unwrap();
        assert_eq!(req.channel, "telegram");
        assert!(req.created);

        let req_again = db
            .upsert_pairing_request("telegram", "user_case", None)
            .await
            .unwrap();
        assert_ne!(req_again.code, req.code);
        assert!(req_again.created);

        let pending = db.list_pending_pairings("TELEGRAM").await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].channel, "telegram");

        db.approve_pairing("TeLeGrAm", &req_again.code, "alice")
            .await
            .unwrap();

        let identity = db
            .resolve_channel_identity("TELEGRAM", "user_case")
            .await
            .unwrap()
            .expect("identity should resolve");
        assert_eq!(identity.owner_id.as_str(), "alice");

        let conn = db.connect().await.unwrap();
        let mut rows = conn
            .query(
                "SELECT channel FROM channel_identities WHERE external_id = ?1",
                libsql::params!["user_case"],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().expect("channel identity row");
        let stored_channel: String = row.get(0).unwrap();
        assert_eq!(stored_channel, "telegram");
    }

    #[tokio::test]
    async fn test_read_allow_from_returns_only_active_paired_external_ids() {
        let (db, _dir) = setup_db().await;

        for (id, status) in [("active-owner", "active"), ("inactive-owner", "suspended")] {
            db.get_or_create_user(UserRecord {
                id: id.to_string(),
                role: "member".to_string(),
                display_name: id.to_string(),
                status: status.to_string(),
                email: None,
                last_login_at: None,
                created_by: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                metadata: serde_json::Value::Null,
                max_agents: None,
                max_tokens: None,
            })
            .await
            .unwrap();
        }

        let active_req = db
            .upsert_pairing_request("telegram", "active-user", None)
            .await
            .unwrap();
        let inactive_req = db
            .upsert_pairing_request("telegram", "inactive-user", None)
            .await
            .unwrap();

        db.approve_pairing("telegram", &active_req.code, "active-owner")
            .await
            .unwrap();
        db.approve_pairing("telegram", &inactive_req.code, "inactive-owner")
            .await
            .unwrap();

        let allowed = db.read_allow_from("TeLeGrAm").await.unwrap();
        assert_eq!(allowed, vec!["active-user".to_string()]);
    }
}
