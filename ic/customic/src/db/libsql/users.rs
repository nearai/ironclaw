//! UserStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{fmt_opt_ts, fmt_ts, get_opt_text, get_opt_ts, get_text, get_ts, opt_text};
use crate::db::libsql::LibSqlBackend;
use crate::db::{ApiTokenRecord, DatabaseError, UserRecord, UserStore};

fn row_to_user(row: &libsql::Row) -> Result<UserRecord, DatabaseError> {
    let metadata_str = get_text(row, 9);
    let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    Ok(UserRecord {
        id: get_text(row, 0),
        email: get_opt_text(row, 1),
        display_name: get_text(row, 2),
        status: get_text(row, 3),
        role: get_text(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
        last_login_at: get_opt_ts(row, 7),
        created_by: get_opt_text(row, 8),
        metadata,
    })
}

fn row_to_api_token(row: &libsql::Row) -> Result<ApiTokenRecord, DatabaseError> {
    let id_str = get_text(row, 0);
    let id: Uuid = id_str
        .parse()
        .map_err(|e| DatabaseError::Serialization(format!("invalid UUID: {e}")))?;
    Ok(ApiTokenRecord {
        id,
        user_id: get_text(row, 1),
        name: get_text(row, 2),
        token_prefix: get_text(row, 3),
        expires_at: get_opt_ts(row, 4),
        last_used_at: get_opt_ts(row, 5),
        created_at: get_ts(row, 6),
        revoked_at: get_opt_ts(row, 7),
    })
}

#[async_trait]
impl UserStore for LibSqlBackend {
    async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let metadata_json = serde_json::to_string(&user.metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            r#"
            INSERT INTO users (id, email, display_name, status, role, created_at, updated_at, last_login_at, created_by, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
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
                SELECT id, email, display_name, status, role, created_at, updated_at,
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
                SELECT id, email, display_name, status, role, created_at, updated_at,
                       last_login_at, created_by, metadata
                FROM users WHERE LOWER(email) = LOWER(?1)
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
                SELECT id, email, display_name, status, role, created_at, updated_at,
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
                SELECT id, email, display_name, status, role, created_at, updated_at,
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

    async fn update_user_role(&self, id: &str, role: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE users SET role = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, role, now],
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
            tokens.push(row_to_api_token(&row)?);
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
                    u.id, u.email, u.display_name, u.status, u.role, u.created_at,
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
                let id_str = get_text(&row, 0);
                let token_id: Uuid = id_str
                    .parse()
                    .map_err(|e| DatabaseError::Serialization(format!("invalid UUID: {e}")))?;
                let token = ApiTokenRecord {
                    id: token_id,
                    user_id: get_text(&row, 1),
                    name: get_text(&row, 2),
                    token_prefix: get_text(&row, 3),
                    expires_at: get_opt_ts(&row, 4),
                    last_used_at: get_opt_ts(&row, 5),
                    created_at: get_ts(&row, 6),
                    revoked_at: get_opt_ts(&row, 7),
                };

                let metadata_str = get_text(&row, 17);
                let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
                    .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

                let user = UserRecord {
                    id: get_text(&row, 8),
                    email: get_opt_text(&row, 9),
                    display_name: get_text(&row, 10),
                    status: get_text(&row, 11),
                    role: get_text(&row, 12),
                    created_at: get_ts(&row, 13),
                    updated_at: get_ts(&row, 14),
                    last_login_at: get_opt_ts(&row, 15),
                    created_by: get_opt_text(&row, 16),
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

    async fn delete_user(&self, id: &str) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            // Delete from child tables first to avoid FK violations.
            // agent_jobs cascades to job_actions, llm_calls, estimation_snapshots
            // conversations cascades to conversation_messages
            // memory_documents cascades to memory_chunks
            // routines cascades to routine_runs
            for table in &[
                "settings",
                "heartbeat_state",
                "tool_rate_limit_state",
                "secret_usage_log",
                "leak_detection_events",
                "secrets",
                "wasm_tools",
                "routines",
                "memory_documents",
                "conversations",
                "api_tokens",
            ] {
                conn.execute(
                    &format!("DELETE FROM {} WHERE user_id = ?1", table),
                    params![id],
                )
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
            // job_events references agent_jobs(id) without CASCADE — delete via subquery.
            conn.execute(
                "DELETE FROM job_events WHERE job_id IN (SELECT id FROM agent_jobs WHERE user_id = ?1)",
                params![id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            conn.execute("DELETE FROM agent_jobs WHERE user_id = ?1", params![id])
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            // Nullify self-referencing created_by before deleting the user
            conn.execute(
                "UPDATE users SET created_by = NULL WHERE created_by = ?1",
                params![id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
            let rows = conn
                .execute("DELETE FROM users WHERE id = ?1", params![id])
                .await
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
            Ok::<_, DatabaseError>(rows > 0)
        }
        .await;

        match result {
            Ok(deleted) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                Ok(deleted)
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }

    async fn user_usage_stats(
        &self,
        user_id: Option<&str>,
        since: DateTime<Utc>,
    ) -> Result<Vec<crate::db::UserUsageStats>, DatabaseError> {
        let conn = self.connect().await?;
        let since_str = fmt_ts(&since);
        let mut rows = if let Some(uid) = user_id {
            conn.query(
                r#"
                SELECT COALESCE(j.user_id, c.user_id) as user_id,
                       l.model, COUNT(*) as call_count,
                       COALESCE(SUM(l.input_tokens), 0) as input_tokens,
                       COALESCE(SUM(l.output_tokens), 0) as output_tokens,
                       CAST(COALESCE(SUM(l.cost), 0) AS TEXT) as total_cost
                FROM llm_calls l
                LEFT JOIN agent_jobs j ON l.job_id = j.id
                LEFT JOIN conversations c ON l.conversation_id = c.id
                WHERE l.created_at >= ?1
                  AND COALESCE(j.user_id, c.user_id) = ?2
                GROUP BY COALESCE(j.user_id, c.user_id), l.model
                ORDER BY total_cost DESC
                "#,
                params![since_str, uid],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                r#"
                SELECT COALESCE(j.user_id, c.user_id) as user_id,
                       l.model, COUNT(*) as call_count,
                       COALESCE(SUM(l.input_tokens), 0) as input_tokens,
                       COALESCE(SUM(l.output_tokens), 0) as output_tokens,
                       CAST(COALESCE(SUM(l.cost), 0) AS TEXT) as total_cost
                FROM llm_calls l
                LEFT JOIN agent_jobs j ON l.job_id = j.id
                LEFT JOIN conversations c ON l.conversation_id = c.id
                WHERE l.created_at >= ?1
                GROUP BY COALESCE(j.user_id, c.user_id), l.model
                ORDER BY total_cost DESC
                "#,
                params![since_str],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };
        let mut stats = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let cost_str = get_text(&row, 5);
            let total_cost = rust_decimal::Decimal::from_str_exact(&cost_str).map_err(|e| {
                DatabaseError::Query(format!("invalid cost value '{}': {}", cost_str, e))
            })?;
            stats.push(crate::db::UserUsageStats {
                user_id: get_text(&row, 0),
                model: get_text(&row, 1),
                call_count: row
                    .get::<i64>(2)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
                input_tokens: row
                    .get::<i64>(3)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
                output_tokens: row
                    .get::<i64>(4)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
                total_cost,
            });
        }
        Ok(stats)
    }

    async fn user_summary_stats(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<crate::db::UserSummaryStats>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(uid) = user_id {
            conn.query(
                r#"
                SELECT
                    COALESCE(j.user_id, c.user_id) AS user_id,
                    COUNT(DISTINCT j.id) AS job_count,
                    CAST(COALESCE(SUM(l.cost), 0) AS TEXT) AS total_cost,
                    MAX(l.created_at) AS last_active_at
                FROM llm_calls l
                LEFT JOIN agent_jobs j ON l.job_id = j.id
                LEFT JOIN conversations c ON l.conversation_id = c.id
                WHERE COALESCE(j.user_id, c.user_id) = ?1
                GROUP BY COALESCE(j.user_id, c.user_id)
                "#,
                params![uid],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                r#"
                SELECT
                    COALESCE(j.user_id, c.user_id) AS user_id,
                    COUNT(DISTINCT j.id) AS job_count,
                    CAST(COALESCE(SUM(l.cost), 0) AS TEXT) AS total_cost,
                    MAX(l.created_at) AS last_active_at
                FROM llm_calls l
                LEFT JOIN agent_jobs j ON l.job_id = j.id
                LEFT JOIN conversations c ON l.conversation_id = c.id
                GROUP BY COALESCE(j.user_id, c.user_id)
                "#,
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };
        let mut stats = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let cost_str = get_text(&row, 2);
            let total_cost = rust_decimal::Decimal::from_str_exact(&cost_str).map_err(|e| {
                DatabaseError::Query(format!("invalid cost value '{}': {}", cost_str, e))
            })?;
            stats.push(crate::db::UserSummaryStats {
                user_id: get_text(&row, 0),
                job_count: row
                    .get::<i64>(1)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?,
                total_cost,
                last_active_at: get_opt_ts(&row, 3),
            });
        }
        Ok(stats)
    }

    async fn create_user_with_token(
        &self,
        user: &UserRecord,
        token_name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError> {
        let conn = self.connect().await?;
        let metadata_json = serde_json::to_string(&user.metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // Insert user
        if let Err(e) = conn
            .execute(
                r#"
                INSERT INTO users (id, email, display_name, status, role, created_at, updated_at, last_login_at, created_by, metadata)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
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
                    metadata_json,
                ],
            )
            .await
        {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(DatabaseError::Query(e.to_string()));
        }

        // Insert token
        let id = Uuid::new_v4();
        let now = Utc::now();
        if let Err(e) = conn
            .execute(
                r#"
                INSERT INTO api_tokens (id, user_id, token_hash, token_prefix, name, expires_at, created_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
                params![
                    id.to_string(),
                    user.id.as_str(),
                    libsql::Value::Blob(token_hash.to_vec()),
                    token_prefix,
                    token_name,
                    fmt_opt_ts(&expires_at),
                    fmt_ts(&now),
                ],
            )
            .await
        {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(DatabaseError::Query(e.to_string()));
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(ApiTokenRecord {
            id,
            user_id: user.id.clone(),
            name: token_name.to_string(),
            token_prefix: token_prefix.to_string(),
            expires_at,
            last_used_at: None,
            created_at: now,
            revoked_at: None,
        })
    }
}
