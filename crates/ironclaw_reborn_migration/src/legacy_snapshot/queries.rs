//! The 7 v1 read queries this crate needs, frozen from
//! `src/history/store.rs` (Postgres) and `src/db/libsql/*.rs` (libSQL) —
//! the entire `Database` trait method surface this crate ever called
//! (`list_conversations_all_channels`, `list_conversation_messages`,
//! `list_all_routines`, `list_documents`, `list_agent_jobs`,
//! `list_identities_for_user`, `get_all_settings`). `Database` itself is a
//! 9-sub-trait, ~78-method supertrait that cannot be partially implemented as
//! a trait object — rather than port the whole thing, [`LegacyDb`] is a
//! concrete enum with only these 7 inherent methods.

use std::collections::HashMap;

use uuid::Uuid;

use super::connect::LegacyDb;
use super::error::LegacyError;
use super::types::normalize_notify_user;
use super::types::{
    AgentJobRecord, Conversation, ConversationMessage, MemoryDocument, NotifyConfig,
    RoutineGuardrails, Trigger, UserIdentityRecord,
};
use super::types::{Routine, RoutineAction};

use crate::source::is_missing_postgres_table_error;
use crate::source::is_missing_table_error;

/// Explicit column list for the `routines` table — libSQL positional reads
/// match this order 1:1; Postgres reads `SELECT *` by name, so this is
/// libSQL-only (see [`super::connect::ensure_schema_current`] for the
/// independent, backend-agnostic expected-column list used to detect a stale
/// source schema).
pub(crate) const ROUTINE_COLUMNS: &str = "\
    id, name, description, user_id, enabled, \
    trigger_type, trigger_config, action_type, action_config, \
    cooldown_secs, max_concurrent, dedup_window_secs, \
    notify_channel, notify_user, notify_on_success, notify_on_failure, notify_on_attention, \
    state, last_run_at, next_fire_at, run_count, consecutive_failures, \
    created_at, updated_at";

impl LegacyDb {
    pub(crate) async fn list_conversations_all_channels(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<Conversation>, LegacyError> {
        match self {
            LegacyDb::LibSql(db) => libsql_conversations(db, user_id, limit).await,
            LegacyDb::Postgres(pool) => postgres_conversations(pool, user_id, limit).await,
        }
    }

    pub(crate) async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, LegacyError> {
        match self {
            LegacyDb::LibSql(db) => libsql_conversation_messages(db, conversation_id).await,
            LegacyDb::Postgres(pool) => postgres_conversation_messages(pool, conversation_id).await,
        }
    }

    pub(crate) async fn list_all_routines(&self) -> Result<Vec<Routine>, LegacyError> {
        match self {
            LegacyDb::LibSql(db) => libsql_all_routines(db).await,
            LegacyDb::Postgres(pool) => postgres_all_routines(pool).await,
        }
    }

    pub(crate) async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, LegacyError> {
        match self {
            LegacyDb::LibSql(db) => libsql_documents(db, user_id, agent_id).await,
            LegacyDb::Postgres(pool) => postgres_documents(pool, user_id, agent_id).await,
        }
    }

    pub(crate) async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, LegacyError> {
        match self {
            LegacyDb::LibSql(db) => libsql_agent_jobs(db).await,
            LegacyDb::Postgres(pool) => postgres_agent_jobs(pool).await,
        }
    }

    pub(crate) async fn list_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserIdentityRecord>, LegacyError> {
        match self {
            LegacyDb::LibSql(db) => libsql_identities_for_user(db, user_id).await,
            LegacyDb::Postgres(pool) => postgres_identities_for_user(pool, user_id).await,
        }
    }

    pub(crate) async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, LegacyError> {
        match self {
            LegacyDb::LibSql(db) => libsql_all_settings(db, user_id).await,
            LegacyDb::Postgres(pool) => postgres_all_settings(pool, user_id).await,
        }
    }
}

// ============================== libSQL ======================================

use crate::legacy_snapshot::libsql_helpers::{
    get_i64, get_json, get_opt_text, get_opt_ts, get_text, get_ts,
};

async fn libsql_connect(
    db: &std::sync::Arc<libsql::Database>,
) -> Result<libsql::Connection, LegacyError> {
    let conn = db
        .connect()
        .map_err(|e| LegacyError::Connect(e.to_string()))?;
    conn.query("PRAGMA busy_timeout = 5000", ())
        .await
        .map_err(|e| LegacyError::Connect(e.to_string()))?;
    Ok(conn)
}

async fn libsql_conversations(
    db: &std::sync::Arc<libsql::Database>,
    user_id: &str,
    limit: i64,
) -> Result<Vec<Conversation>, LegacyError> {
    let conn = libsql_connect(db).await?;
    let mut rows = conn
        .query(
            r#"
            SELECT
                c.id,
                c.started_at,
                c.last_activity,
                c.metadata,
                c.channel,
                (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id AND m.role = 'user') AS message_count,
                (SELECT substr(m2.content, 1, 100)
                 FROM conversation_messages m2
                 WHERE m2.conversation_id = c.id AND m2.role = 'user'
                 ORDER BY m2.created_at ASC, m2.rowid ASC
                 LIMIT 1
                ) AS title
            FROM conversations c
            WHERE c.user_id = ?1
            ORDER BY datetime(c.last_activity) DESC
            LIMIT ?2
            "#,
            libsql::params![user_id, limit],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    let mut results = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?
    {
        let metadata = get_json(&row, 3);
        let thread_type = metadata
            .get("thread_type")
            .and_then(|v| v.as_str())
            .map(String::from);
        let sql_title = get_opt_text(&row, 6);
        let title = sql_title.or_else(|| {
            metadata
                .get("routine_name")
                .and_then(|v| v.as_str())
                .map(String::from)
        });
        results.push(Conversation {
            id: get_text(&row, 0).parse().unwrap_or_default(),
            title,
            channel: get_text(&row, 4),
            thread_type,
            started_at: get_ts(&row, 1),
            last_activity: get_ts(&row, 2),
        });
    }
    Ok(results)
}

async fn libsql_conversation_messages(
    db: &std::sync::Arc<libsql::Database>,
    conversation_id: Uuid,
) -> Result<Vec<ConversationMessage>, LegacyError> {
    let conn = libsql_connect(db).await?;
    let mut rows = conn
        .query(
            r#"
            SELECT id, role, content, created_at
            FROM conversation_messages
            WHERE conversation_id = ?1
            ORDER BY created_at ASC, rowid ASC
            "#,
            libsql::params![conversation_id.to_string()],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    let mut messages = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?
    {
        messages.push(ConversationMessage {
            id: get_text(&row, 0).parse().unwrap_or_default(),
            role: get_text(&row, 1),
            content: get_text(&row, 2),
            created_at: get_ts(&row, 3),
        });
    }
    Ok(messages)
}

async fn libsql_all_routines(
    db: &std::sync::Arc<libsql::Database>,
) -> Result<Vec<Routine>, LegacyError> {
    let conn = libsql_connect(db).await?;
    let mut rows = conn
        .query(
            &format!("SELECT {ROUTINE_COLUMNS} FROM routines ORDER BY name"),
            (),
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    let mut routines = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?
    {
        routines.push(libsql_row_to_routine(&row)?);
    }
    Ok(routines)
}

fn libsql_row_to_routine(row: &libsql::Row) -> Result<Routine, LegacyError> {
    let trigger_type = get_text(row, 5);
    let trigger_config = get_json(row, 6);
    let action_type = get_text(row, 7);
    let action_config = get_json(row, 8);
    let cooldown_secs = get_i64(row, 9);
    let max_concurrent = get_i64(row, 10);
    let dedup_window_secs: Option<i64> = row.get::<i64>(11).ok();

    let trigger = Trigger::from_db(&trigger_type, trigger_config)?;
    let action = RoutineAction::from_db(&action_type, action_config)?;

    Ok(Routine {
        id: get_text(row, 0).parse().unwrap_or_default(),
        name: get_text(row, 1),
        description: get_text(row, 2),
        user_id: get_text(row, 3),
        enabled: get_i64(row, 4) != 0,
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(cooldown_secs as u64),
            max_concurrent: max_concurrent as u32,
            dedup_window: dedup_window_secs.map(|s| std::time::Duration::from_secs(s as u64)),
        },
        notify: NotifyConfig {
            channel: get_opt_text(row, 12),
            user: normalize_notify_user(get_opt_text(row, 13)),
            on_success: get_i64(row, 14) != 0,
            on_failure: get_i64(row, 15) != 0,
            on_attention: get_i64(row, 16) != 0,
        },
        state: get_json(row, 17),
        last_run_at: get_opt_ts(row, 18),
        next_fire_at: get_opt_ts(row, 19),
        run_count: get_i64(row, 20) as u64,
        consecutive_failures: get_i64(row, 21) as u32,
        created_at: get_ts(row, 22),
        updated_at: get_ts(row, 23),
    })
}

async fn libsql_documents(
    db: &std::sync::Arc<libsql::Database>,
    user_id: &str,
    agent_id: Option<Uuid>,
) -> Result<Vec<MemoryDocument>, LegacyError> {
    let conn = libsql_connect(db).await?;
    let agent_id_str = agent_id.map(|id| id.to_string());
    let mut rows = conn
        .query(
            r#"
            SELECT id, user_id, agent_id, path, content,
                   created_at, updated_at, metadata
            FROM memory_documents
            WHERE user_id = ?1 AND agent_id IS ?2
            ORDER BY updated_at DESC
            "#,
            libsql::params![user_id, agent_id_str.as_deref()],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    let mut docs = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?
    {
        docs.push(MemoryDocument {
            id: get_text(&row, 0).parse().unwrap_or_default(),
            user_id: get_text(&row, 1),
            agent_id: get_opt_text(&row, 2).and_then(|s| s.parse().ok()),
            path: get_text(&row, 3),
            content: get_text(&row, 4),
            created_at: get_ts(&row, 5),
            updated_at: get_ts(&row, 6),
            metadata: get_json(&row, 7),
        });
    }
    Ok(docs)
}

async fn libsql_agent_jobs(
    db: &std::sync::Arc<libsql::Database>,
) -> Result<Vec<AgentJobRecord>, LegacyError> {
    let conn = libsql_connect(db).await?;
    let mut rows = conn
        .query(
            r#"
            SELECT id, title, status, user_id, failure_reason,
                   created_at, started_at, completed_at
            FROM agent_jobs WHERE source = 'direct'
            ORDER BY created_at DESC
            "#,
            (),
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    let mut jobs = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?
    {
        let id_str = get_text(&row, 0);
        let Ok(id) = id_str.parse() else {
            tracing::warn!("Skipping legacy agent job with invalid UUID: {}", id_str);
            continue;
        };
        jobs.push(AgentJobRecord {
            id,
            title: get_text(&row, 1),
            status: get_text(&row, 2),
            user_id: get_text(&row, 3),
            failure_reason: get_opt_text(&row, 4),
            created_at: get_ts(&row, 5),
            started_at: get_opt_ts(&row, 6),
            completed_at: get_opt_ts(&row, 7),
        });
    }
    Ok(jobs)
}

async fn libsql_identities_for_user(
    db: &std::sync::Arc<libsql::Database>,
    user_id: &str,
) -> Result<Vec<UserIdentityRecord>, LegacyError> {
    let conn = libsql_connect(db).await?;
    let mut rows = match conn
        .query(
            "SELECT id, user_id, provider, provider_user_id, email, email_verified, \
             display_name, avatar_url, raw_profile, created_at, updated_at \
             FROM user_identities WHERE user_id = ?1 ORDER BY created_at",
            libsql::params![user_id],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) if is_missing_table_error(&e.to_string()) => return Ok(Vec::new()),
        Err(e) => return Err(LegacyError::Query(e.to_string())),
    };

    let mut result = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?
    {
        result.push(libsql_row_to_identity(&row)?);
    }
    Ok(result)
}

fn libsql_row_to_identity(row: &libsql::Row) -> Result<UserIdentityRecord, LegacyError> {
    let id_str = get_text(row, 0);
    let id: Uuid = id_str
        .parse()
        .map_err(|e: uuid::Error| LegacyError::Decode {
            what: "user_identities.id".into(),
            field: e.to_string(),
        })?;
    let raw_str = get_text(row, 8);
    let raw_profile: serde_json::Value =
        serde_json::from_str(&raw_str).map_err(|e| LegacyError::Decode {
            what: "user_identities.raw_profile".into(),
            field: e.to_string(),
        })?;
    Ok(UserIdentityRecord {
        id,
        user_id: get_text(row, 1),
        provider: get_text(row, 2),
        provider_user_id: get_text(row, 3),
        email: get_opt_text(row, 4),
        email_verified: get_i64(row, 5) != 0,
        display_name: get_opt_text(row, 6),
        avatar_url: get_opt_text(row, 7),
        raw_profile,
        created_at: get_ts(row, 9),
        updated_at: get_ts(row, 10),
    })
}

async fn libsql_all_settings(
    db: &std::sync::Arc<libsql::Database>,
    user_id: &str,
) -> Result<HashMap<String, serde_json::Value>, LegacyError> {
    let conn = libsql_connect(db).await?;
    let mut rows = conn
        .query(
            "SELECT key, value FROM settings WHERE user_id = ?1",
            libsql::params![user_id],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    let mut map = HashMap::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?
    {
        map.insert(get_text(&row, 0), get_json(&row, 1));
    }
    Ok(map)
}

// ============================== PostgreSQL ==================================

async fn pg_client(
    pool: &deadpool_postgres::Pool,
) -> Result<deadpool_postgres::Client, LegacyError> {
    pool.get()
        .await
        .map_err(|e| LegacyError::Connect(e.to_string()))
}

async fn postgres_conversations(
    pool: &deadpool_postgres::Pool,
    user_id: &str,
    limit: i64,
) -> Result<Vec<Conversation>, LegacyError> {
    let client = pg_client(pool).await?;
    let rows = client
        .query(
            r#"
            SELECT
                c.id,
                c.started_at,
                c.last_activity,
                c.metadata,
                c.channel,
                (SELECT COUNT(*) FROM conversation_messages m WHERE m.conversation_id = c.id AND m.role = 'user') AS message_count,
                (SELECT LEFT(m2.content, 100)
                 FROM conversation_messages m2
                 WHERE m2.conversation_id = c.id AND m2.role = 'user'
                 ORDER BY m2.created_at ASC
                 LIMIT 1
                ) AS title
            FROM conversations c
            WHERE c.user_id = $1
            ORDER BY c.last_activity DESC
            LIMIT $2
            "#,
            &[&user_id, &limit],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    Ok(rows
        .iter()
        .map(|r| {
            let metadata: serde_json::Value = r.get("metadata");
            let thread_type = metadata
                .get("thread_type")
                .and_then(|v| v.as_str())
                .map(String::from);
            let sql_title: Option<String> = r.get("title");
            let title = sql_title.or_else(|| {
                metadata
                    .get("routine_name")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });
            Conversation {
                id: r.get("id"),
                title,
                channel: r.get("channel"),
                thread_type,
                started_at: r.get("started_at"),
                last_activity: r.get("last_activity"),
            }
        })
        .collect())
}

async fn postgres_conversation_messages(
    pool: &deadpool_postgres::Pool,
    conversation_id: Uuid,
) -> Result<Vec<ConversationMessage>, LegacyError> {
    let client = pg_client(pool).await?;
    let rows = client
        .query(
            r#"
            SELECT id, role, content, created_at
            FROM conversation_messages
            WHERE conversation_id = $1
            ORDER BY created_at ASC
            "#,
            &[&conversation_id],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    Ok(rows
        .iter()
        .map(|r| ConversationMessage {
            id: r.get("id"),
            role: r.get("role"),
            content: r.get("content"),
            created_at: r.get("created_at"),
        })
        .collect())
}

async fn postgres_all_routines(
    pool: &deadpool_postgres::Pool,
) -> Result<Vec<Routine>, LegacyError> {
    let client = pg_client(pool).await?;
    let rows = client
        .query("SELECT * FROM routines ORDER BY name", &[])
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;
    rows.iter().map(postgres_row_to_routine).collect()
}

fn postgres_row_to_routine(row: &tokio_postgres::Row) -> Result<Routine, LegacyError> {
    let trigger_type: String = row.get("trigger_type");
    let trigger_config: serde_json::Value = row.get("trigger_config");
    let action_type: String = row.get("action_type");
    let action_config: serde_json::Value = row.get("action_config");
    let cooldown_secs: i32 = row.get("cooldown_secs");
    let max_concurrent: i32 = row.get("max_concurrent");
    let dedup_window_secs: Option<i32> = row.get("dedup_window_secs");

    let trigger = Trigger::from_db(&trigger_type, trigger_config)?;
    let action = RoutineAction::from_db(&action_type, action_config)?;

    Ok(Routine {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        user_id: row.get("user_id"),
        enabled: row.get("enabled"),
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(cooldown_secs as u64),
            max_concurrent: max_concurrent as u32,
            dedup_window: dedup_window_secs.map(|s| std::time::Duration::from_secs(s as u64)),
        },
        notify: NotifyConfig {
            channel: row.get("notify_channel"),
            user: row.get("notify_user"),
            on_attention: row.get("notify_on_attention"),
            on_failure: row.get("notify_on_failure"),
            on_success: row.get("notify_on_success"),
        },
        last_run_at: row.get("last_run_at"),
        next_fire_at: row.get("next_fire_at"),
        run_count: row.get::<_, i64>("run_count") as u64,
        consecutive_failures: row.get::<_, i32>("consecutive_failures") as u32,
        state: row.get("state"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

async fn postgres_documents(
    pool: &deadpool_postgres::Pool,
    user_id: &str,
    agent_id: Option<Uuid>,
) -> Result<Vec<MemoryDocument>, LegacyError> {
    let client = pg_client(pool).await?;
    let rows = client
        .query(
            r#"
            SELECT id, user_id, agent_id, path, content,
                   created_at, updated_at, metadata
            FROM memory_documents
            WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2
            ORDER BY updated_at DESC
            "#,
            &[&user_id, &agent_id],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    Ok(rows
        .iter()
        .map(|r| MemoryDocument {
            id: r.get("id"),
            user_id: r.get("user_id"),
            agent_id: r.get("agent_id"),
            path: r.get("path"),
            content: r.get("content"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
            metadata: r.get("metadata"),
        })
        .collect())
}

async fn postgres_agent_jobs(
    pool: &deadpool_postgres::Pool,
) -> Result<Vec<AgentJobRecord>, LegacyError> {
    let client = pg_client(pool).await?;
    let rows = client
        .query(
            r#"
            SELECT id, title, status, user_id, failure_reason,
                   created_at, started_at, completed_at
            FROM agent_jobs WHERE source = 'direct'
            ORDER BY created_at DESC
            "#,
            &[],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;

    Ok(rows
        .iter()
        .map(|r| AgentJobRecord {
            id: r.get("id"),
            title: r.get("title"),
            status: r.get("status"),
            user_id: r.get::<_, Option<String>>("user_id").unwrap_or_default(),
            failure_reason: r.get("failure_reason"),
            created_at: r.get("created_at"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
        })
        .collect())
}

async fn postgres_identities_for_user(
    pool: &deadpool_postgres::Pool,
    user_id: &str,
) -> Result<Vec<UserIdentityRecord>, LegacyError> {
    let client = pg_client(pool).await?;
    let rows = match client
        .query(
            "SELECT id, user_id, provider, provider_user_id, email, email_verified, \
             display_name, avatar_url, raw_profile, created_at, updated_at \
             FROM user_identities WHERE user_id = $1 ORDER BY created_at",
            &[&user_id],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) if is_missing_postgres_table_error(&e) => return Ok(Vec::new()),
        Err(e) => return Err(LegacyError::Query(e.to_string())),
    };

    Ok(rows
        .iter()
        .map(|row| UserIdentityRecord {
            id: row.get("id"),
            user_id: row.get("user_id"),
            provider: row.get("provider"),
            provider_user_id: row.get("provider_user_id"),
            email: row.get("email"),
            email_verified: row.get("email_verified"),
            display_name: row.get("display_name"),
            avatar_url: row.get("avatar_url"),
            raw_profile: row.get("raw_profile"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

async fn postgres_all_settings(
    pool: &deadpool_postgres::Pool,
    user_id: &str,
) -> Result<HashMap<String, serde_json::Value>, LegacyError> {
    let client = pg_client(pool).await?;
    let rows = client
        .query(
            "SELECT key, value FROM settings WHERE user_id = $1",
            &[&user_id],
        )
        .await
        .map_err(|e| LegacyError::Query(e.to_string()))?;
    Ok(rows
        .iter()
        .map(|r| {
            let key: String = r.get("key");
            let value: serde_json::Value = r.get("value");
            (key, value)
        })
        .collect())
}
