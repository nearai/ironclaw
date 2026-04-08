//! Database abstraction layer.
//!
//! Provides a backend-agnostic `Database` trait that unifies all persistence
//! operations. Two implementations exist behind feature flags:
//!
//! - `postgres` (default): Uses `deadpool-postgres` + `tokio-postgres`
//! - `libsql`: Uses libSQL (Turso's SQLite fork) for embedded/edge deployment
//!
//! The existing `Store`, `Repository`, `SecretsStore`, and `WasmToolStore`
//! types become thin wrappers that delegate to `Arc<dyn Database>`.

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "postgres")]
pub mod tls;

#[cfg(feature = "libsql")]
pub mod libsql;

#[cfg(feature = "libsql")]
pub mod libsql_migrations;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::agent::BrokenTool;
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::context::{ActionRecord, JobContext, JobState};
use crate::error::DatabaseError;
use crate::error::WorkspaceError;
use crate::history::{
    AgentJobRecord, AgentJobSummary, ConversationMessage, ConversationSummary, JobEventRecord,
    LlmCallRecord, SandboxJobRecord, SandboxJobSummary, SettingRow,
};
use crate::workspace::{MemoryChunk, MemoryDocument, WorkspaceEntry};
use crate::workspace::{SearchConfig, SearchResult};

/// Create a database backend from configuration, run migrations, and return it.
///
/// This is the shared helper for CLI commands and other call sites that need
/// a simple `Arc<dyn Database>` without retaining backend-specific handles
/// (e.g., `pg_pool` or `libsql_conn` for the secrets store). The main agent
/// startup in `main.rs` uses its own initialization block because it also
/// captures those backend-specific handles.
pub async fn connect_from_config(
    config: &crate::config::DatabaseConfig,
) -> Result<Arc<dyn Database>, DatabaseError> {
    let (db, _handles) = connect_with_handles(config).await?;
    Ok(db)
}

/// Backend-specific handles retained after database connection.
///
/// These are needed by satellite stores (e.g., `SecretsStore`) that require
/// a backend-specific handle rather than the generic `Arc<dyn Database>`.
#[derive(Default)]
pub struct DatabaseHandles {
    #[cfg(feature = "postgres")]
    pub pg_pool: Option<deadpool_postgres::Pool>,
    #[cfg(feature = "libsql")]
    pub libsql_db: Option<Arc<::libsql::Database>>,
}

/// Connect to the database, run migrations, and return both the generic
/// `Database` trait object and the backend-specific handles.
pub async fn connect_with_handles(
    config: &crate::config::DatabaseConfig,
) -> Result<(Arc<dyn Database>, DatabaseHandles), DatabaseError> {
    let mut handles = DatabaseHandles::default();

    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };
            backend.run_migrations().await?;
            tracing::debug!("libSQL database connected and migrations applied");

            handles.libsql_db = Some(backend.shared_db());

            Ok((Arc::new(backend) as Arc<dyn Database>, handles))
        }
        #[cfg(feature = "postgres")]
        crate::config::DatabaseBackend::Postgres => {
            let pg = postgres::PgBackend::new(config)
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?;
            pg.run_migrations().await?;
            tracing::info!("PostgreSQL database connected and migrations applied");

            handles.pg_pool = Some(pg.pool());

            Ok((Arc::new(pg) as Arc<dyn Database>, handles))
        }
        #[allow(unreachable_patterns)]
        _ => Err(DatabaseError::Pool(format!(
            "Database backend '{}' is not available. Rebuild with the appropriate feature flag.",
            config.backend
        ))),
    }
}

/// Create a secrets store from database and secrets configuration.
///
/// This is the shared factory for CLI commands and other call sites that need
/// a `SecretsStore` without going through the full `AppBuilder`. Mirrors the
/// pattern of [`connect_from_config`] but returns a secrets-specific store.
pub async fn create_secrets_store(
    config: &crate::config::DatabaseConfig,
    crypto: Arc<crate::secrets::SecretsCrypto>,
) -> Result<Arc<dyn crate::secrets::SecretsStore + Send + Sync>, DatabaseError> {
    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };
            backend.run_migrations().await?;

            Ok(Arc::new(crate::secrets::LibSqlSecretsStore::new(
                backend.shared_db(),
                crypto,
            )))
        }
        #[cfg(feature = "postgres")]
        crate::config::DatabaseBackend::Postgres => {
            let pg = postgres::PgBackend::new(config)
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?;
            pg.run_migrations().await?;

            Ok(Arc::new(crate::secrets::PostgresSecretsStore::new(
                pg.pool(),
                crypto,
            )))
        }
        #[allow(unreachable_patterns)]
        _ => Err(DatabaseError::Pool(format!(
            "Database backend '{}' is not available for secrets. Rebuild with the appropriate feature flag.",
            config.backend
        ))),
    }
}

// ==================== Wizard / testing helpers ====================

/// Connect to the database WITHOUT running migrations, validating
/// prerequisites when applicable (PostgreSQL version, pgvector).
///
/// Returns both the `Database` trait object and backend-specific handles.
/// Used by the wizard to test connectivity before committing — call
/// [`Database::run_migrations`] on the returned trait object when ready.
pub async fn connect_without_migrations(
    config: &crate::config::DatabaseConfig,
) -> Result<(Arc<dyn Database>, DatabaseHandles), DatabaseError> {
    let mut handles = DatabaseHandles::default();

    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };

            handles.libsql_db = Some(backend.shared_db());

            Ok((Arc::new(backend) as Arc<dyn Database>, handles))
        }
        #[cfg(feature = "postgres")]
        crate::config::DatabaseBackend::Postgres => {
            let pg = postgres::PgBackend::new(config)
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?;

            handles.pg_pool = Some(pg.pool());

            // Validate PostgreSQL prerequisites (version, pgvector)
            validate_postgres(&pg.pool()).await?;

            Ok((Arc::new(pg) as Arc<dyn Database>, handles))
        }
        #[allow(unreachable_patterns)]
        _ => Err(DatabaseError::Pool(format!(
            "Database backend '{}' is not available. Rebuild with the appropriate feature flag.",
            config.backend
        ))),
    }
}

/// Validate PostgreSQL prerequisites (version >= 15, pgvector available).
///
/// Returns `Ok(())` if all prerequisites are met, or a `DatabaseError`
/// with a user-facing message describing the issue.
#[cfg(feature = "postgres")]
async fn validate_postgres(pool: &deadpool_postgres::Pool) -> Result<(), DatabaseError> {
    let client = pool
        .get()
        .await
        .map_err(|e| DatabaseError::Pool(format!("Failed to connect: {}", e)))?;

    // Check PostgreSQL server version (need 15+ for pgvector).
    let version_row = client
        .query_one("SHOW server_version", &[])
        .await
        .map_err(|e| DatabaseError::Query(format!("Failed to query server version: {}", e)))?;
    let version_str: &str = version_row.get(0);
    let major_version = version_str
        .split('.')
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| {
            DatabaseError::Pool(format!(
                "Could not parse PostgreSQL version from '{}'. \
                 Expected a numeric major version (e.g., '15.2').",
                version_str
            ))
        })?;

    const MIN_PG_MAJOR_VERSION: u32 = 15;

    if major_version < MIN_PG_MAJOR_VERSION {
        return Err(DatabaseError::Pool(format!(
            "PostgreSQL {} detected. IronClaw requires PostgreSQL {} or later \
             for pgvector support.\n\
             Upgrade: https://www.postgresql.org/download/",
            version_str, MIN_PG_MAJOR_VERSION
        )));
    }

    // Check if pgvector extension is available.
    let pgvector_row = client
        .query_opt(
            "SELECT 1 FROM pg_available_extensions WHERE name = 'vector'",
            &[],
        )
        .await
        .map_err(|e| {
            DatabaseError::Query(format!("Failed to check pgvector availability: {}", e))
        })?;

    if pgvector_row.is_none() {
        return Err(DatabaseError::Pool(format!(
            "pgvector extension not found on your PostgreSQL server.\n\n\
             Install it:\n  \
             macOS:   brew install pgvector\n  \
             Ubuntu:  apt install postgresql-{0}-pgvector\n  \
             Docker:  use the pgvector/pgvector:pg{0} image\n  \
             Source:  https://github.com/pgvector/pgvector#installation\n\n\
             Then restart PostgreSQL and re-run: ironclaw onboard",
            major_version
        )));
    }

    Ok(())
}

// ==================== User management record types ====================

/// A registered user.
#[derive(Debug, Clone)]
pub struct UserRecord {
    /// User identifier (string, matches existing `user_id` throughout the codebase).
    pub id: String,
    pub email: Option<String>,
    pub display_name: String,
    /// `active`, `suspended`, or `deactivated`.
    pub status: String,
    /// `admin` or `member`.
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    /// Who created/invited this user (nullable for bootstrap users).
    pub created_by: Option<String>,
    pub metadata: serde_json::Value,
}

/// An API token for authenticating requests (hash stored, never plaintext).
#[derive(Debug, Clone)]
pub struct ApiTokenRecord {
    pub id: Uuid,
    pub user_id: String,
    /// Human label (e.g. "my-laptop", "ci-bot").
    pub name: String,
    /// First 8 hex chars of the plaintext token for display/identification.
    pub token_prefix: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Soft-revoke timestamp. Non-null means revoked.
    pub revoked_at: Option<DateTime<Utc>>,
}

// ==================== User identity record types ====================

/// A linked external identity from an OAuth/social login provider.
#[derive(Debug, Clone)]
pub struct UserIdentityRecord {
    pub id: Uuid,
    pub user_id: String,
    /// Provider name (e.g. `google`, `github`, `apple`, `near`, `email`).
    pub provider: String,
    /// Provider-specific unique user identifier (Google `sub`, GitHub user ID, etc.).
    pub provider_user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Raw JSON profile payload from the provider for debugging/auditing.
    pub raw_profile: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ==================== Sub-traits ====================
//
// Each sub-trait groups related persistence methods. The `Database` supertrait
// combines them all, so existing `Arc<dyn Database>` consumers keep working.
// Leaf consumers can depend on a specific sub-trait instead.

#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError>;
    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError>;
    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError>;
    /// Insert a message only if the conversation has zero messages.
    /// Returns `Ok(true)` if the message was inserted, `Ok(false)` if skipped.
    async fn add_conversation_message_if_empty(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<bool, DatabaseError>;
    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
        source_channel: Option<&str>,
    ) -> Result<bool, DatabaseError>;
    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    async fn list_conversations_all_channels(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError>;
    /// Read-only lookup for an existing routine conversation. Returns `None`
    /// if the routine has never executed (no conversation created yet).
    async fn find_routine_conversation(
        &self,
        routine_id: Uuid,
        user_id: &str,
    ) -> Result<Option<Uuid>, DatabaseError>;
    async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError>;
    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError>;
    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError>;
    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;
    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError>;
    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;
    /// Get the source_channel for a conversation (the channel that created it).
    async fn get_conversation_source_channel(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<String>, DatabaseError>;
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError>;
    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError>;
    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError>;
    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError>;
    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError>;
    async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, DatabaseError>;
    async fn list_agent_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<AgentJobRecord>, DatabaseError>;
    async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError>;
    async fn agent_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<AgentJobSummary, DatabaseError>;
    /// Get the failure reason for a single agent job (O(1) lookup).
    async fn get_agent_job_failure_reason(&self, id: Uuid)
    -> Result<Option<String>, DatabaseError>;
    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError>;
    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError>;
    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError>;
    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError>;
    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait SandboxStore: Send + Sync {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError>;
    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError>;
    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError>;
    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError>;
    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError>;
    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError>;
    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError>;
    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError>;
    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError>;
    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError>;
    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError>;
}

#[async_trait]
pub trait RoutineStore: Send + Sync {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;
    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError>;
    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError>;
    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError>;
    async fn list_all_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;
    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError>;
    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError>;
    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError>;
    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError>;
    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError>;
    async fn count_running_routine_runs_batch(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, i64>, DatabaseError>;

    /// Fetch the last run status for multiple routines in a single query.
    /// Returns a map from routine_id to its most recent RunStatus.
    /// Routines with no runs are omitted from the result.
    async fn batch_get_last_run_status(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, RunStatus>, DatabaseError>;

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError>;
    async fn get_webhook_routine_by_path(
        &self,
        path: &str,
        user_id: Option<&str>,
    ) -> Result<Option<Routine>, DatabaseError>;

    /// List routine runs that were dispatched as full_job but have not yet
    /// been finalized (status='running' with a linked job_id).
    async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError>;
}

#[async_trait]
pub trait ToolFailureStore: Send + Sync {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError>;
    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError>;
    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError>;
    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait SettingsStore: Send + Sync {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;
    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError>;
    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError>;
    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError>;
    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError>;
    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError>;
    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError>;
    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError>;
    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError>;
    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError>;
    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError>;
    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError>;
    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError>;
    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;
    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError>;
    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError>;

    // ==================== Metadata ====================
    //
    // **Trust boundary:** methods in this section accept bare document/version
    // UUIDs without a `user_id` guard. They trust the caller (`Workspace`) to
    // have obtained the UUID through a user-scoped query (e.g.
    // `get_document_by_path` or `get_or_create_document_by_path`). Do NOT call
    // these with an unverified UUID from external input.

    /// Update the metadata JSON field on a document (full replacement).
    ///
    /// # Trust
    /// Caller must have verified ownership of `id` via a user-scoped lookup.
    async fn update_document_metadata(
        &self,
        id: Uuid,
        metadata: &serde_json::Value,
    ) -> Result<(), WorkspaceError>;

    /// Find all `.config` documents in the workspace.
    ///
    /// Returns documents whose path ends with `/.config` or equals `.config`.
    /// Used by the hygiene system to discover metadata-driven cleanup targets.
    async fn find_config_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError>;

    // ==================== Versioning ====================
    //
    // **Trust boundary:** same as metadata — these accept bare `document_id`
    // UUIDs and trust the caller to have verified ownership first.

    /// Save the current content of a document as a new version.
    ///
    /// Returns the new version number (1-based, monotonically increasing).
    ///
    /// # Trust
    /// Caller must have verified ownership of `document_id`.
    async fn save_version(
        &self,
        document_id: Uuid,
        content: &str,
        content_hash: &str,
        changed_by: Option<&str>,
    ) -> Result<i32, WorkspaceError>;

    /// Get a specific version of a document.
    ///
    /// # Trust
    /// Caller must have verified ownership of `document_id`.
    async fn get_version(
        &self,
        document_id: Uuid,
        version: i32,
    ) -> Result<crate::workspace::DocumentVersion, WorkspaceError>;

    /// List versions of a document (newest first).
    ///
    /// # Trust
    /// Caller must have verified ownership of `document_id`.
    async fn list_versions(
        &self,
        document_id: Uuid,
        limit: i64,
    ) -> Result<Vec<crate::workspace::VersionSummary>, WorkspaceError>;

    /// Get the latest version number for a document, or `None` if no versions exist.
    ///
    /// # Trust
    /// Caller must have verified ownership of `document_id`.
    async fn get_latest_version_number(
        &self,
        document_id: Uuid,
    ) -> Result<Option<i32>, WorkspaceError>;

    /// Delete old versions, keeping only the most recent `keep_count`.
    ///
    /// Returns the number of versions deleted.
    ///
    /// # Trust
    /// Caller must have verified ownership of `document_id`.
    async fn prune_versions(
        &self,
        document_id: Uuid,
        keep_count: i32,
    ) -> Result<u64, WorkspaceError>;

    // ==================== Multi-scope read methods ====================
    //
    // Default implementations loop over user_ids calling single-scope methods,
    // then merge results. Backends can override with efficient SQL (e.g.,
    // `WHERE user_id = ANY($1::text[])`).

    /// Hybrid search across multiple user scopes, merging results by score.
    ///
    /// **Note:** The default implementation calls `hybrid_search` per scope and
    /// merges by raw score. Because RRF scores are normalized independently
    /// within each scope, scores are not directly comparable across scopes.
    /// The Postgres backend overrides this with a single combined query that
    /// applies RRF once to the unified result set.
    async fn hybrid_search_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        if user_ids.len() > 1 {
            tracing::debug!(
                scope_count = user_ids.len(),
                "hybrid_search_multi: using default per-scope RRF merge; \
                 cross-scope score comparison may be unreliable"
            );
        }
        let mut all_results = Vec::new();
        for uid in user_ids {
            let results = self
                .hybrid_search(uid, agent_id, query, embedding, config)
                .await?;
            all_results.extend(results);
        }
        // Re-sort by score descending and truncate to limit
        all_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_results.truncate(config.limit);
        Ok(all_results)
    }

    /// List all file paths across multiple user scopes.
    async fn list_all_paths_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let mut all_paths = Vec::new();
        for uid in user_ids {
            let paths = self.list_all_paths(uid, agent_id).await?;
            all_paths.extend(paths);
        }
        all_paths.sort();
        all_paths.dedup();
        Ok(all_paths)
    }

    /// Get a document by path, searching across multiple user scopes.
    ///
    /// Returns the first match found (tries each user_id in order).
    async fn get_document_by_path_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        for uid in user_ids {
            match self.get_document_by_path(uid, agent_id, path).await {
                Ok(doc) => return Ok(doc),
                Err(WorkspaceError::DocumentNotFound { .. }) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(WorkspaceError::DocumentNotFound {
            doc_type: path.to_string(),
            user_id: format!("[{}]", user_ids.join(", ")),
        })
    }

    /// List directory contents across multiple user scopes.
    async fn list_directory_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let mut all_entries = Vec::new();
        for uid in user_ids {
            all_entries.extend(self.list_directory(uid, agent_id, directory).await?);
        }
        Ok(crate::workspace::merge_workspace_entries(all_entries))
    }
}

#[async_trait]
pub trait UserStore: Send + Sync {
    // ---- Users ----

    /// Create a new user record.
    async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError>;

    /// Create the user if they do not already exist. Idempotent.
    ///
    /// Each backend must override this with an atomic upsert (PostgreSQL:
    /// `ON CONFLICT DO NOTHING`; libSQL: `INSERT OR IGNORE`) to avoid the
    /// TOCTOU race in a SELECT + INSERT sequence.
    async fn get_or_create_user(&self, user: UserRecord) -> Result<(), DatabaseError>;
    /// Get a user by their string id.
    async fn get_user(&self, id: &str) -> Result<Option<UserRecord>, DatabaseError>;
    /// Get a user by email address.
    async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DatabaseError>;
    /// List users, optionally filtered by status.
    async fn list_users(&self, status: Option<&str>) -> Result<Vec<UserRecord>, DatabaseError>;
    /// Update a user's status (active/suspended/deactivated).
    async fn update_user_status(&self, id: &str, status: &str) -> Result<(), DatabaseError>;
    /// Update a user's role (admin/member).
    async fn update_user_role(&self, id: &str, role: &str) -> Result<(), DatabaseError>;
    /// Update a user's display name and metadata.
    async fn update_user_profile(
        &self,
        id: &str,
        display_name: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    /// Record a login timestamp.
    async fn record_login(&self, id: &str) -> Result<(), DatabaseError>;

    // ---- API Tokens ----

    /// Create a new API token. The `token_hash` is SHA-256 of the plaintext.
    async fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError>;
    /// List tokens for a user (never includes the hash).
    async fn list_api_tokens(&self, user_id: &str) -> Result<Vec<ApiTokenRecord>, DatabaseError>;
    /// Soft-revoke a token. Returns false if the token doesn't exist or doesn't belong to the user.
    async fn revoke_api_token(&self, token_id: Uuid, user_id: &str) -> Result<bool, DatabaseError>;
    /// Look up a token by hash, returning the token record and its owning user.
    /// Only returns active (non-revoked, non-expired) tokens for active users.
    async fn authenticate_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<(ApiTokenRecord, UserRecord)>, DatabaseError>;
    /// Update `last_used_at` for a token.
    async fn record_token_usage(&self, token_id: Uuid) -> Result<(), DatabaseError>;

    /// Check whether any user records exist (for first-run bootstrap detection).
    async fn has_any_users(&self) -> Result<bool, DatabaseError>;

    /// Delete a user and all their data across all user-scoped tables.
    /// Returns false if the user doesn't exist.
    async fn delete_user(&self, id: &str) -> Result<bool, DatabaseError>;

    /// Get per-user LLM usage stats for a time period.
    /// Aggregates from llm_calls via agent_jobs.user_id.
    async fn user_usage_stats(
        &self,
        user_id: Option<&str>,
        since: DateTime<Utc>,
    ) -> Result<Vec<UserUsageStats>, DatabaseError>;

    /// Lightweight per-user summary stats (job count, total cost, last active).
    /// Used by the admin users list to show inline stats.
    async fn user_summary_stats(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<UserSummaryStats>, DatabaseError>;

    /// Create a user and their initial API token atomically.
    /// If either operation fails, both are rolled back.
    async fn create_user_with_token(
        &self,
        user: &UserRecord,
        token_name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError>;
}

/// Per-user LLM usage statistics.
#[derive(Debug, Clone)]
pub struct UserUsageStats {
    pub user_id: String,
    pub model: String,
    pub call_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_cost: Decimal,
}

/// Lightweight per-user summary for the admin users list.
#[derive(Debug, Clone)]
pub struct UserSummaryStats {
    pub user_id: String,
    /// Total agent jobs created by this user.
    pub job_count: i64,
    /// Total LLM spend across all jobs (all-time).
    pub total_cost: Decimal,
    /// Most recent activity (latest job or LLM call timestamp).
    pub last_active_at: Option<DateTime<Utc>>,
}

/// A pending pairing request.
#[derive(Debug, Clone)]
pub struct PairingRequestRecord {
    pub id: uuid::Uuid,
    pub channel: String,
    pub external_id: String,
    pub code: String,
    pub created: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Pairing and channel identity operations.
/// Named `ChannelPairingStore` to avoid collision with the application-level
/// `PairingStore` struct in `src/pairing/store.rs`.
#[async_trait]
pub trait ChannelPairingStore: Send + Sync {
    /// Returns the `Identity` for `(channel, external_id)` if the sender has been paired.
    /// Joins `channel_identities` with `users` to get OwnerId + UserRole in one query.
    async fn resolve_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<Option<crate::ownership::Identity>, DatabaseError>;

    /// Read paired external IDs for a channel, for compatibility with legacy
    /// allow-list-based WASM channel admission.
    async fn read_allow_from(&self, channel: &str) -> Result<Vec<String>, DatabaseError>;

    /// Create or replace the pending pairing request for `(channel, external_id)`.
    /// Any existing non-expired pending request for the same sender is retired and a new code
    /// is issued so retrying the claim flow always rotates to a fresh code.
    async fn upsert_pairing_request(
        &self,
        channel: &str,
        external_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<PairingRequestRecord, DatabaseError>;

    /// Approve the pairing `code`, mapping `(channel, external_id)` → `owner_id`.
    /// Sets owner_id on the pairing_requests row + creates channel_identities row — one transaction.
    /// Returns `Err` if code is invalid, expired, already approved, or belongs to a different channel.
    async fn approve_pairing(
        &self,
        channel: &str,
        code: &str,
        owner_id: &str,
    ) -> Result<(), DatabaseError>;

    /// List pending (unapproved, non-expired) pairing requests for a channel.
    async fn list_pending_pairings(
        &self,
        channel: &str,
    ) -> Result<Vec<PairingRequestRecord>, DatabaseError>;

    /// Remove a channel identity (unlink a channel from a user).
    async fn remove_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<(), DatabaseError>;
}

/// Generates an 8-character pairing code from an unambiguous alphabet.
pub fn generate_pairing_code() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::rngs::OsRng;
    (0..8)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Persistence for linked external identities (OAuth/social login providers).
#[async_trait]
pub trait IdentityStore: Send + Sync {
    /// Find a user identity by provider and provider-specific user ID.
    async fn get_identity_by_provider(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError>;

    /// Find all identities linked to a user.
    async fn list_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserIdentityRecord>, DatabaseError>;

    /// Create a new identity link.
    async fn create_identity(&self, identity: &UserIdentityRecord) -> Result<(), DatabaseError>;

    /// Update display_name and avatar_url on an existing identity (e.g. on re-login).
    async fn update_identity_profile(
        &self,
        provider: &str,
        provider_user_id: &str,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
    ) -> Result<(), DatabaseError>;

    /// Find identities with a given verified email (for automatic account linking).
    async fn find_identity_by_verified_email(
        &self,
        email: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError>;

    /// Create a new user and link an identity atomically.
    async fn create_user_with_identity(
        &self,
        user: &UserRecord,
        identity: &UserIdentityRecord,
    ) -> Result<(), DatabaseError>;
}

/// Backend-agnostic database supertrait.
///
/// Combines all sub-traits into one. Existing `Arc<dyn Database>` consumers
/// continue to work; leaf consumers can depend on a specific sub-trait instead.
#[async_trait]
pub trait Database:
    ConversationStore
    + JobStore
    + SandboxStore
    + RoutineStore
    + ToolFailureStore
    + SettingsStore
    + WorkspaceStore
    + UserStore
    + ChannelPairingStore
    + IdentityStore
    + Send
    + Sync
{
    /// Run schema migrations for this backend.
    async fn run_migrations(&self) -> Result<(), DatabaseError>;

    /// Rewrite all rows where user_id = 'default' to owner_id across all
    /// affected tables. Idempotent — safe to call on every startup.
    async fn migrate_default_owner(&self, owner_id: &str) -> Result<(), DatabaseError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test: `create_secrets_store` selects the correct backend at
    /// runtime based on `DatabaseConfig`, not at compile time. Previously the
    /// CLI duplicated this logic with compile-time `#[cfg]` gates that always
    /// chose postgres when both features were enabled (PR #209).
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_create_secrets_store_libsql_backend() {
        use secrecy::SecretString;

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");

        let config = crate::config::DatabaseConfig {
            backend: crate::config::DatabaseBackend::LibSql,
            libsql_path: Some(db_path),
            libsql_url: None,
            libsql_auth_token: None,
            url: SecretString::from("unused://libsql".to_string()),
            pool_size: 1,
            ssl_mode: crate::config::SslMode::default(),
        };

        let master_key = SecretString::from("a]".repeat(16));
        let crypto = Arc::new(crate::secrets::SecretsCrypto::new(master_key).unwrap());

        let store = create_secrets_store(&config, crypto).await;
        assert!(
            store.is_ok(),
            "create_secrets_store should succeed for libsql backend"
        );

        // Verify basic operation works
        let store = store.unwrap();
        let exists = store.exists("test_user", "nonexistent_secret").await;
        assert!(exists.is_ok());
        assert!(!exists.unwrap());
    }
}
