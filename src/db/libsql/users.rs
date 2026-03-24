//! UserStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{fmt_opt_ts, fmt_ts, get_opt_text, get_opt_ts, get_text, get_ts, opt_text};
use crate::db::libsql::LibSqlBackend;
use crate::db::{ApiTokenRecord, DatabaseError, InvitationRecord, UserRecord, UserStore};

fn row_to_user(row: &libsql::Row) -> Result<UserRecord, DatabaseError> {
    let metadata_str = get_text(row, 8);
    let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    Ok(UserRecord {
        id: get_text(row, 0),
        email: get_opt_text(row, 1),
        display_name: get_text(row, 2),
        status: get_text(row, 3),
        created_at: get_ts(row, 4),
        updated_at: get_ts(row, 5),
        last_login_at: get_opt_ts(row, 6),
        created_by: get_opt_text(row, 7),
        metadata,
    })
}

fn row_to_api_token(row: &libsql::Row) -> ApiTokenRecord {
    ApiTokenRecord {
        id: get_text(row, 0).parse().unwrap_or_default(),
        user_id: get_text(row, 1),
        name: get_text(row, 2),
        token_prefix: get_text(row, 3),
        expires_at: get_opt_ts(row, 4),
        last_used_at: get_opt_ts(row, 5),
        created_at: get_ts(row, 6),
        revoked_at: get_opt_ts(row, 7),
    }
}

fn row_to_invitation(row: &libsql::Row) -> InvitationRecord {
    InvitationRecord {
        id: get_text(row, 0).parse().unwrap_or_default(),
        email: get_opt_text(row, 1),
        invited_by: get_text(row, 2),
        status: get_text(row, 3),
        expires_at: get_ts(row, 4),
        accepted_at: get_opt_ts(row, 5),
        accepted_by: get_opt_text(row, 6),
        created_at: get_ts(row, 7),
    }
}

#[async_trait]
impl UserStore for LibSqlBackend {
    async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let metadata_json = serde_json::to_string(&user.metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
            INSERT INTO users (id, email, display_name, status, created_at, updated_at, last_login_at, created_by, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                user.id.as_str(),
                opt_text(user.email.as_deref()),
                user.display_name.as_str(),
                user.status.as_str(),
                fmt_ts(&user.created_at),
                fmt_ts(&user.updated_at),
                fmt_opt_ts(&user.last_login_at),
                opt_text(user.created_by.as_deref()),
                metadata_json,
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_user(&self, id: &str) -> Result<Option<UserRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, email, display_name, status, created_at, updated_at,
                       last_login_at, created_by, metadata
                FROM users WHERE id = ?1
                "#,
                params![id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_user(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, email, display_name, status, created_at, updated_at,
                       last_login_at, created_by, metadata
                FROM users WHERE email = ?1
                "#,
                params![email],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_user(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_users(&self, status: Option<&str>) -> Result<Vec<UserRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut users = Vec::new();

        let mut rows = if let Some(status) = status {
            conn.query(
                r#"
                SELECT id, email, display_name, status, created_at, updated_at,
                       last_login_at, created_by, metadata
                FROM users WHERE status = ?1
                ORDER BY created_at DESC
                "#,
                params![status],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                r#"
                SELECT id, email, display_name, status, created_at, updated_at,
                       last_login_at, created_by, metadata
                FROM users
                ORDER BY created_at DESC
                "#,
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            users.push(row_to_user(&row)?);
        }
        Ok(users)
    }

    async fn update_user_status(&self, id: &str, status: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE users SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status, now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn update_user_profile(
        &self,
        id: &str,
        display_name: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let metadata_json = serde_json::to_string(metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        conn.execute(
            "UPDATE users SET display_name = ?2, metadata = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, display_name, metadata_json, now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn record_login(&self, id: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE users SET last_login_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4();
        let now = Utc::now();

        conn.execute(
            r#"
            INSERT INTO api_tokens (id, user_id, token_hash, token_prefix, name, expires_at, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                id.to_string(),
                user_id,
                libsql::Value::Blob(token_hash.to_vec()),
                token_prefix,
                name,
                fmt_opt_ts(&expires_at),
                fmt_ts(&now),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(ApiTokenRecord {
            id,
            user_id: user_id.to_string(),
            name: name.to_string(),
            token_prefix: token_prefix.to_string(),
            expires_at,
            last_used_at: None,
            created_at: now,
            revoked_at: None,
        })
    }

    async fn list_api_tokens(&self, user_id: &str) -> Result<Vec<ApiTokenRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, token_prefix, expires_at, last_used_at, created_at, revoked_at
                FROM api_tokens WHERE user_id = ?1
                ORDER BY created_at DESC
                "#,
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut tokens = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            tokens.push(row_to_api_token(&row));
        }
        Ok(tokens)
    }

    async fn revoke_api_token(&self, token_id: Uuid, user_id: &str) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let rows_affected = conn
            .execute(
                r#"
                UPDATE api_tokens SET revoked_at = ?3
                WHERE id = ?1 AND user_id = ?2 AND revoked_at IS NULL
                "#,
                params![token_id.to_string(), user_id, now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows_affected > 0)
    }

    async fn authenticate_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<(ApiTokenRecord, UserRecord)>, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());

        let mut rows = conn
            .query(
                r#"
                SELECT
                    t.id, t.user_id, t.name, t.token_prefix, t.expires_at,
                    t.last_used_at, t.created_at, t.revoked_at,
                    u.id, u.email, u.display_name, u.status, u.created_at,
                    u.updated_at, u.last_login_at, u.created_by, u.metadata
                FROM api_tokens t
                JOIN users u ON u.id = t.user_id
                WHERE t.token_hash = ?1
                  AND t.revoked_at IS NULL
                  AND (t.expires_at IS NULL OR t.expires_at > ?2)
                  AND u.status = 'active'
                "#,
                params![libsql::Value::Blob(token_hash.to_vec()), now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => {
                let token = ApiTokenRecord {
                    id: get_text(&row, 0).parse().unwrap_or_default(),
                    user_id: get_text(&row, 1),
                    name: get_text(&row, 2),
                    token_prefix: get_text(&row, 3),
                    expires_at: get_opt_ts(&row, 4),
                    last_used_at: get_opt_ts(&row, 5),
                    created_at: get_ts(&row, 6),
                    revoked_at: get_opt_ts(&row, 7),
                };

                let metadata_str = get_text(&row, 16);
                let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
                    .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

                let user = UserRecord {
                    id: get_text(&row, 8),
                    email: get_opt_text(&row, 9),
                    display_name: get_text(&row, 10),
                    status: get_text(&row, 11),
                    created_at: get_ts(&row, 12),
                    updated_at: get_ts(&row, 13),
                    last_login_at: get_opt_ts(&row, 14),
                    created_by: get_opt_text(&row, 15),
                    metadata,
                };

                Ok(Some((token, user)))
            }
            None => Ok(None),
        }
    }

    async fn record_token_usage(&self, token_id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE api_tokens SET last_used_at = ?2 WHERE id = ?1",
            params![token_id.to_string(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn create_invitation(
        &self,
        invitation: &InvitationRecord,
        invite_hash: &[u8; 32],
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO invitations (id, email, invite_token_hash, invited_by, status, expires_at, accepted_at, accepted_by, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                invitation.id.to_string(),
                opt_text(invitation.email.as_deref()),
                libsql::Value::Blob(invite_hash.to_vec()),
                invitation.invited_by.as_str(),
                invitation.status.as_str(),
                fmt_ts(&invitation.expires_at),
                fmt_opt_ts(&invitation.accepted_at),
                opt_text(invitation.accepted_by.as_deref()),
                fmt_ts(&invitation.created_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_invitation_by_hash(
        &self,
        invite_hash: &[u8; 32],
    ) -> Result<Option<InvitationRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, email, invited_by, status, expires_at, accepted_at, accepted_by, created_at
                FROM invitations WHERE invite_token_hash = ?1
                "#,
                params![libsql::Value::Blob(invite_hash.to_vec())],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_invitation(&row))),
            None => Ok(None),
        }
    }

    async fn accept_invitation(&self, id: Uuid, accepted_by: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            r#"
            UPDATE invitations SET status = 'accepted', accepted_at = ?2, accepted_by = ?3
            WHERE id = ?1
            "#,
            params![id.to_string(), now, accepted_by],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn list_invitations(
        &self,
        invited_by: Option<&str>,
    ) -> Result<Vec<InvitationRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut invitations = Vec::new();

        let mut rows = if let Some(invited_by) = invited_by {
            conn.query(
                r#"
                SELECT id, email, invited_by, status, expires_at, accepted_at, accepted_by, created_at
                FROM invitations WHERE invited_by = ?1
                ORDER BY created_at DESC
                "#,
                params![invited_by],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                r#"
                SELECT id, email, invited_by, status, expires_at, accepted_at, accepted_by, created_at
                FROM invitations
                ORDER BY created_at DESC
                "#,
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            invitations.push(row_to_invitation(&row));
        }
        Ok(invitations)
    }

    async fn has_any_users(&self) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query("SELECT 1 FROM users LIMIT 1", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let has_users = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .is_some();
        Ok(has_users)
    }
}
