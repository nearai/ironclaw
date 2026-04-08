//! UserStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::params;
use uuid::Uuid;

use super::{fmt_opt_ts, fmt_ts, get_opt_text, get_opt_ts, get_text, get_ts, opt_text};
use crate::db::libsql::LibSqlBackend;
use crate::db::{ApiTokenRecord, DatabaseError, UserRecord, UserStore};
use crate::workspace::GREETING_SEED;

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

pub(crate) async fn seed_initial_assistant_thread(
    conn: &libsql::Connection,
    user_id: &str,
    created_at: &DateTime<Utc>,
) -> Result<(), DatabaseError> {
    let conversation_id = Uuid::new_v4();
    let message_id = Uuid::new_v4();
    let started_at = fmt_ts(created_at);
    let metadata = serde_json::json!({
        "thread_type": "assistant",
        "title": "Assistant",
    });

    conn.execute(
        "INSERT INTO conversations (id, channel, user_id, metadata, source_channel, started_at, last_activity) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            conversation_id.to_string(),
            "gateway",
            user_id,
            metadata.to_string(),
            "gateway",
            started_at.clone(),
        ],
    )
    .await
    .map_err(|e| DatabaseError::Query(e.to_string()))?;

    conn.execute(
        "INSERT INTO conversation_messages (id, conversation_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            message_id.to_string(),
            conversation_id.to_string(),
            "assistant",
            GREETING_SEED,
            started_at,
        ],
    )
    .await
    .map_err(|e| DatabaseError::Query(e.to_string()))?;

    Ok(())
}

#[async_trait]
impl UserStore for LibSqlBackend {
    async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let metadata_json = serde_json::to_string(&user.metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute("BEGIN", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if let Err(err) = async {
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

            seed_initial_assistant_thread(&conn, &user.id, &user.created_at).await
        }
        .await
        {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(err);
        }

        conn.execute("COMMIT", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn get_or_create_user(&self, user: UserRecord) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let metadata_json = serde_json::to_string(&user.metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        // Wrap the (insert + seed assistant thread) cycle in a transaction
        // so a seed failure rolls back the user row, preserving the
        // invariant that every provisioned user has a seeded assistant
        // thread (matches the pattern in `create_user`).
        conn.execute("BEGIN", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let result = async {
            let rows = conn
                .execute(
                    r#"
                INSERT OR IGNORE INTO users (id, email, display_name, status, role, created_at, updated_at, last_login_at, created_by, metadata)
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
                .map_err(|e| DatabaseError::Query(format!("get_or_create_user: {e}")))?;

            if rows > 0 {
                seed_initial_assistant_thread(&conn, &user.id, &user.created_at).await?;
            }
            Ok::<_, DatabaseError>(())
        }
        .await;

        if let Err(err) = result {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(err);
        }

        conn.execute("COMMIT", ())
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

        if let Err(e) = seed_initial_assistant_thread(&conn, &user.id, &user.created_at).await {
            let _ = conn.execute("ROLLBACK", ()).await;
            return Err(e);
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

    async fn user_summary_stats(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<crate::db::UserSummaryStats>, DatabaseError> {
        let conn = self.connect().await?;
        // Aggregate from llm_calls, resolving user_id via either agent_jobs
        // (for background job calls) or conversations (for chat calls where
        // job_id is NULL). Also count distinct agent_jobs per user.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::libsql::LibSqlBackend;
    use crate::db::{ConversationStore, Database, UserStore};
    use sha2::{Digest, Sha256};

    fn hash(s: &str) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(s.as_bytes());
        h.finalize().into()
    }

    async fn setup() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_users.db");
        let db = LibSqlBackend::new_local(&db_path).await.unwrap();
        db.run_migrations().await.unwrap();
        (db, dir) // keep dir alive so the DB file isn't deleted
    }

    fn test_user(id: &str) -> UserRecord {
        UserRecord {
            id: id.to_string(),
            email: Some(format!("{}@test.com", id)),
            display_name: id.to_string(),
            status: "active".to_string(),
            role: "member".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        }
    }

    async fn assistant_messages(
        db: &LibSqlBackend,
        user_id: &str,
    ) -> Vec<crate::history::ConversationMessage> {
        let thread_id = db
            .get_or_create_assistant_conversation(user_id, "gateway")
            .await
            .unwrap();
        db.list_conversation_messages(thread_id).await.unwrap()
    }

    #[tokio::test]
    async fn test_has_any_users_empty() {
        let (db, _dir) = setup().await;
        assert!(!db.has_any_users().await.unwrap());
    }

    #[tokio::test]
    async fn test_create_and_get_user() {
        let (db, _dir) = setup().await;
        let user = test_user("alice");
        db.create_user(&user).await.unwrap();

        assert!(db.has_any_users().await.unwrap());

        let found = db.get_user("alice").await.unwrap().unwrap();
        assert_eq!(found.id, "alice");
        assert_eq!(found.email, Some("alice@test.com".to_string()));
        assert_eq!(found.status, "active");
    }

    #[tokio::test]
    async fn test_create_user_seeds_initial_assistant_greeting() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();

        let messages = assistant_messages(&db, "alice").await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "assistant");
        assert_eq!(messages[0].content, GREETING_SEED);
    }

    #[tokio::test]
    async fn test_get_or_create_user_seeds_initial_assistant_greeting_on_insert() {
        let (db, _dir) = setup().await;
        let user = test_user("owner");

        db.get_or_create_user(user.clone()).await.unwrap();

        let messages = assistant_messages(&db, "owner").await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, GREETING_SEED);

        db.get_or_create_user(user).await.unwrap();
        let messages_again = assistant_messages(&db, "owner").await;
        assert_eq!(messages_again.len(), 1);
    }

    #[tokio::test]
    async fn test_get_user_by_email() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("bob")).await.unwrap();

        let found = db.get_user_by_email("bob@test.com").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "bob");

        assert!(
            db.get_user_by_email("nobody@test.com")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_list_users_with_status_filter() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();
        db.create_user(&test_user("bob")).await.unwrap();
        db.update_user_status("bob", "suspended").await.unwrap();

        let all = db.list_users(None).await.unwrap();
        assert_eq!(all.len(), 2);

        let active = db.list_users(Some("active")).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "alice");

        let suspended = db.list_users(Some("suspended")).await.unwrap();
        assert_eq!(suspended.len(), 1);
        assert_eq!(suspended[0].id, "bob");
    }

    #[tokio::test]
    async fn test_update_user_profile() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();

        let meta = serde_json::json!({"role": "admin"});
        db.update_user_profile("alice", "Alice Smith", &meta)
            .await
            .unwrap();

        let user = db.get_user("alice").await.unwrap().unwrap();
        assert_eq!(user.display_name, "Alice Smith");
        assert_eq!(user.metadata["role"], "admin");
    }

    #[tokio::test]
    async fn test_token_lifecycle_create_authenticate_revoke() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();

        // Create token
        let token_hash = hash("secret-token-123");
        let record = db
            .create_api_token("alice", "laptop", &token_hash, "secret-t", None)
            .await
            .unwrap();
        assert_eq!(record.user_id, "alice");
        assert_eq!(record.name, "laptop");
        assert_eq!(record.token_prefix, "secret-t");

        // Authenticate
        let (tok, user) = db.authenticate_token(&token_hash).await.unwrap().unwrap();
        assert_eq!(tok.id, record.id);
        assert_eq!(user.id, "alice");

        // List tokens
        let tokens = db.list_api_tokens("alice").await.unwrap();
        assert_eq!(tokens.len(), 1);

        // Revoke
        assert!(db.revoke_api_token(record.id, "alice").await.unwrap());

        // Auth should fail after revoke
        assert!(db.authenticate_token(&token_hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_create_user_with_token_seeds_initial_assistant_greeting() {
        let (db, _dir) = setup().await;
        let user = test_user("token-user");
        let token_hash = hash("bootstrap-token");

        db.create_user_with_token(&user, "initial", &token_hash, "bootstra", None)
            .await
            .unwrap();

        let messages = assistant_messages(&db, "token-user").await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, GREETING_SEED);
    }

    #[tokio::test]
    async fn test_token_auth_fails_for_suspended_user() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();

        let token_hash = hash("token-abc");
        db.create_api_token("alice", "test", &token_hash, "token-ab", None)
            .await
            .unwrap();

        // Auth works while active
        assert!(db.authenticate_token(&token_hash).await.unwrap().is_some());

        // Suspend user
        db.update_user_status("alice", "suspended").await.unwrap();

        // Auth should fail
        assert!(db.authenticate_token(&token_hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_token_revoke_wrong_user_returns_false() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();
        db.create_user(&test_user("bob")).await.unwrap();

        let token_hash = hash("alice-token");
        let record = db
            .create_api_token("alice", "test", &token_hash, "alice-to", None)
            .await
            .unwrap();

        // Bob can't revoke Alice's token
        assert!(!db.revoke_api_token(record.id, "bob").await.unwrap());

        // Alice can
        assert!(db.revoke_api_token(record.id, "alice").await.unwrap());
    }

    #[tokio::test]
    async fn test_record_login_and_token_usage() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();

        let token_hash = hash("tok");
        let record = db
            .create_api_token("alice", "test", &token_hash, "tok", None)
            .await
            .unwrap();

        // Record usage
        db.record_token_usage(record.id).await.unwrap();
        db.record_login("alice").await.unwrap();

        // Verify timestamps updated
        let user = db.get_user("alice").await.unwrap().unwrap();
        assert!(user.last_login_at.is_some());

        let tokens = db.list_api_tokens("alice").await.unwrap();
        assert!(tokens[0].last_used_at.is_some());
    }

    #[tokio::test]
    async fn test_delete_user_removes_api_tokens() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();

        let token_hash = hash("alice-tok");
        db.create_api_token("alice", "primary", &token_hash, "alice-to", None)
            .await
            .unwrap();

        // Verify token exists before deletion.
        let tokens = db.list_api_tokens("alice").await.unwrap();
        assert_eq!(tokens.len(), 1);

        // Delete user — should also remove their api_tokens.
        assert!(db.delete_user("alice").await.unwrap());

        // api_tokens must be gone (not orphaned).
        let tokens = db.list_api_tokens("alice").await.unwrap();
        assert!(
            tokens.is_empty(),
            "expected api_tokens to be deleted with user, found {}",
            tokens.len()
        );
    }

    /// Helper: insert a minimal agent_job row for testing usage stats.
    async fn insert_test_job(db: &LibSqlBackend, job_id: &str, user_id: &str) {
        let conn = db.connect().await.unwrap();
        conn.execute(
            "INSERT INTO agent_jobs (id, title, description, status, source, user_id, created_at) \
             VALUES (?1, 'test', 'test job', 'completed', 'test', ?2, ?3)",
            params![job_id, user_id, fmt_ts(&Utc::now())],
        )
        .await
        .unwrap();
    }

    /// Helper: insert a minimal llm_call row for testing usage stats.
    async fn insert_test_llm_call(db: &LibSqlBackend, job_id: &str, model: &str, cost: &str) {
        let conn = db.connect().await.unwrap();
        let call_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO llm_calls (id, job_id, provider, model, input_tokens, output_tokens, cost, created_at) \
             VALUES (?1, ?2, 'test', ?3, 100, 50, ?4, ?5)",
            params![call_id, job_id, model, cost, fmt_ts(&Utc::now())],
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_user_summary_stats_empty() {
        let (db, _dir) = setup().await;
        let stats = db.user_summary_stats(None).await.unwrap();
        assert!(stats.is_empty());
    }

    #[tokio::test]
    async fn test_user_summary_stats_with_jobs_and_costs() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();
        db.create_user(&test_user("bob")).await.unwrap();

        // Alice: 2 jobs, 2 LLM calls
        insert_test_job(&db, "job-a1", "alice").await;
        insert_test_job(&db, "job-a2", "alice").await;
        insert_test_llm_call(&db, "job-a1", "gpt-4", "0.05").await;
        insert_test_llm_call(&db, "job-a2", "gpt-4", "0.10").await;

        // Bob: 1 job, no LLM calls
        insert_test_job(&db, "job-b1", "bob").await;

        // All users with LLM calls (Bob has a job but no LLM calls, so no stats)
        let stats = db.user_summary_stats(None).await.unwrap();
        assert_eq!(stats.len(), 1);

        let alice_stats = stats.iter().find(|s| s.user_id == "alice").unwrap();
        assert_eq!(alice_stats.job_count, 2);
        assert_eq!(
            alice_stats.total_cost,
            rust_decimal::Decimal::from_str_exact("0.15").unwrap()
        );
        assert!(alice_stats.last_active_at.is_some());

        // Bob has no LLM calls so doesn't appear in summary stats
        assert!(!stats.iter().any(|s| s.user_id == "bob"));

        // Filter to single user
        let alice_only = db.user_summary_stats(Some("alice")).await.unwrap();
        assert_eq!(alice_only.len(), 1);
        assert_eq!(alice_only[0].job_count, 2);

        // Bob returns empty when filtered
        let bob_only = db.user_summary_stats(Some("bob")).await.unwrap();
        assert!(bob_only.is_empty());
    }

    #[tokio::test]
    async fn test_user_usage_stats_with_calls() {
        let (db, _dir) = setup().await;
        db.create_user(&test_user("alice")).await.unwrap();

        insert_test_job(&db, "job-a1", "alice").await;
        insert_test_llm_call(&db, "job-a1", "gpt-4", "0.05").await;
        insert_test_llm_call(&db, "job-a1", "gpt-4", "0.10").await;
        insert_test_llm_call(&db, "job-a1", "gpt-3.5", "0.01").await;

        let since = chrono::Utc::now() - chrono::Duration::hours(1);
        let stats = db.user_usage_stats(None, since).await.unwrap();

        // Two models used
        assert_eq!(stats.len(), 2);

        let gpt4 = stats.iter().find(|s| s.model == "gpt-4").unwrap();
        assert_eq!(gpt4.call_count, 2);
        assert_eq!(gpt4.input_tokens, 200);
        assert_eq!(gpt4.output_tokens, 100);
        assert_eq!(
            gpt4.total_cost,
            rust_decimal::Decimal::from_str_exact("0.15").unwrap()
        );

        let gpt35 = stats.iter().find(|s| s.model == "gpt-3.5").unwrap();
        assert_eq!(gpt35.call_count, 1);
    }
}
