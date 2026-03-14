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
/// Also holds standalone learning-system store trait objects, created from
/// the concrete backend type before type erasure.
#[derive(Default)]
pub struct DatabaseHandles {
    #[cfg(feature = "postgres")]
    pub pg_pool: Option<deadpool_postgres::Pool>,
    #[cfg(feature = "libsql")]
    pub libsql_db: Option<Arc<::libsql::Database>>,
    /// Session search store (learning system).
    pub session_search_store: Option<Arc<dyn SessionSearchStore>>,
    /// User profile store (learning system).
    pub user_profile_store: Option<Arc<dyn UserProfileStore>>,
    /// Learning audit log store (learning system).
    pub learning_store: Option<Arc<dyn LearningStore>>,
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
            tracing::info!("libSQL database connected and migrations applied");

            handles.libsql_db = Some(backend.shared_db());

            // Create standalone store trait objects before type erasure.
            let backend = Arc::new(backend);
            handles.session_search_store = Some(backend.clone() as Arc<dyn SessionSearchStore>);
            handles.user_profile_store = Some(backend.clone() as Arc<dyn UserProfileStore>);
            handles.learning_store = Some(backend.clone() as Arc<dyn LearningStore>);

            Ok((backend as Arc<dyn Database>, handles))
        }
        #[cfg(feature = "postgres")]
        crate::config::DatabaseBackend::Postgres => {
            let pg = postgres::PgBackend::new(config)
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?;
            pg.run_migrations().await?;
            tracing::info!("PostgreSQL database connected and migrations applied");

            handles.pg_pool = Some(pg.pool());

            // Create standalone store trait objects before type erasure.
            let pg = Arc::new(pg);
            handles.session_search_store = Some(pg.clone() as Arc<dyn SessionSearchStore>);
            handles.user_profile_store = Some(pg.clone() as Arc<dyn UserProfileStore>);
            handles.learning_store = Some(pg.clone() as Arc<dyn LearningStore>);

            Ok((pg as Arc<dyn Database>, handles))
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
    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
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
    async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError>;
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
    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError>;
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
}

// ==================== Learning system standalone traits ====================
//
// These are NOT added to the `Database` supertrait to avoid breaking existing
// implementations and test stubs. They are injected separately via `Arc<dyn T>`
// where needed, and created from the concrete backend type before type erasure
// in `connect_with_handles()`.

/// Persistence for session summaries used by the learning system's session search.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait SessionSearchStore: Send + Sync {
    /// Upsert a session summary (insert or update if conversation_id exists).
    async fn upsert_session_summary(
        &self,
        conversation_id: Uuid,
        user_id: &str,
        agent_id: &str,
        summary: &str,
        topics: &[String],
        tool_names: &[String],
        message_count: i32,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, DatabaseError>;

    /// Full-text search over session summaries.
    async fn search_sessions_fts(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SessionSummaryRow>, DatabaseError>;

    /// Vector search over session summary embeddings.
    async fn search_sessions_vector(
        &self,
        user_id: &str,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SessionSummaryRow>, DatabaseError>;

    /// Get a session summary by conversation ID.
    async fn get_session_summary(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<SessionSummaryRow>, DatabaseError>;
}

/// Row type for session summary search results.
#[derive(Debug, Clone)]
pub struct SessionSummaryRow {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub user_id: String,
    pub agent_id: String,
    pub summary: String,
    pub topics: Vec<String>,
    pub tool_names: Vec<String>,
    pub message_count: i32,
    pub created_at: DateTime<Utc>,
    /// Relevance score (FTS rank or cosine similarity).
    pub score: f32,
}

/// Persistence for encrypted user profile facts.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait UserProfileStore: Send + Sync {
    /// Upsert a profile fact (insert or update on conflict).
    async fn upsert_profile_fact(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &str,
        fact_key: &str,
        fact_value_encrypted: &[u8],
        key_salt: &[u8],
        confidence: f32,
        source: &str,
    ) -> Result<Uuid, DatabaseError>;

    /// Get all profile facts for a user.
    async fn get_profile_facts(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Vec<ProfileFactRow>, DatabaseError>;

    /// Get profile facts by category.
    async fn get_profile_facts_by_category(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &str,
    ) -> Result<Vec<ProfileFactRow>, DatabaseError>;

    /// Delete a profile fact by key.
    async fn delete_profile_fact(
        &self,
        user_id: &str,
        agent_id: &str,
        category: &str,
        fact_key: &str,
    ) -> Result<bool, DatabaseError>;
}

/// Row type for encrypted profile facts.
#[derive(Debug, Clone)]
pub struct ProfileFactRow {
    pub id: Uuid,
    pub user_id: String,
    pub agent_id: String,
    pub category: String,
    pub fact_key: String,
    pub fact_value_encrypted: Vec<u8>,
    pub key_salt: Vec<u8>,
    pub confidence: f32,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Status of a synthesized skill in the approval workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillStatus {
    Pending,
    Accepted,
    Rejected,
}

impl SkillStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

impl std::fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Persistence for the synthesized skill audit log.
#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait LearningStore: Send + Sync {
    /// Record a synthesized skill in the audit log.
    async fn record_synthesized_skill(
        &self,
        user_id: &str,
        agent_id: &str,
        skill_name: &str,
        skill_content: Option<&str>,
        content_hash: &str,
        source_conversation_id: Option<Uuid>,
        status: SkillStatus,
        safety_scan_passed: bool,
        quality_score: i32,
    ) -> Result<Uuid, DatabaseError>;

    /// Update the status of a synthesized skill (for approval workflow).
    /// Requires `user_id` to prevent IDOR — only the owner can change status.
    async fn update_synthesized_skill_status(
        &self,
        id: Uuid,
        user_id: &str,
        status: SkillStatus,
    ) -> Result<bool, DatabaseError>;

    /// List synthesized skills by status.
    async fn list_synthesized_skills(
        &self,
        user_id: &str,
        agent_id: &str,
        status: Option<SkillStatus>,
    ) -> Result<Vec<SynthesizedSkillRow>, DatabaseError>;

    /// Get a single synthesized skill by ID.
    /// Requires `user_id` to prevent IDOR — only the owner can read their skills.
    async fn get_synthesized_skill(
        &self,
        id: Uuid,
        user_id: &str,
    ) -> Result<Option<SynthesizedSkillRow>, DatabaseError>;
}

/// Row type for synthesized skill records.
#[derive(Debug, Clone)]
pub struct SynthesizedSkillRow {
    pub id: Uuid,
    pub user_id: String,
    pub agent_id: String,
    pub skill_name: String,
    pub skill_content: Option<String>,
    pub skill_content_hash: String,
    pub source_conversation_id: Option<Uuid>,
    pub status: SkillStatus,
    pub safety_scan_passed: bool,
    pub quality_score: i32,
    pub created_at: DateTime<Utc>,
    pub reviewed_at: Option<DateTime<Utc>>,
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
    + Send
    + Sync
{
    /// Run schema migrations for this backend.
    async fn run_migrations(&self) -> Result<(), DatabaseError>;
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
