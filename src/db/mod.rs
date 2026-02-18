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

#[cfg(feature = "libsql")]
pub mod libsql_backend;

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
    ConversationMessage, ConversationSummary, JobEventRecord, LlmCallRecord, SandboxJobRecord,
    SandboxJobSummary, SettingRow,
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
                libsql_backend::LibSqlBackend::new_remote_replica(
                    db_path,
                    url,
                    token.expose_secret(),
                )
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql_backend::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };
            backend.run_migrations().await?;
            Ok(Arc::new(backend))
        }
        #[cfg(feature = "postgres")]
        _ => {
            let pg = postgres::PgBackend::new(config)
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?;
            pg.run_migrations().await?;
            Ok(Arc::new(pg))
        }
        #[cfg(not(feature = "postgres"))]
        _ => Err(DatabaseError::Pool(
            "No database backend available. Enable 'postgres' or 'libsql' feature.".to_string(),
        )),
    }
}

/// Backend-agnostic database trait.
///
/// Combines all persistence operations from Store, Repository, and related
/// stores into a single trait that can be implemented for different backends.
#[async_trait]
pub trait Database: Send + Sync {
    /// Run schema migrations for this backend.
    async fn run_migrations(&self) -> Result<(), DatabaseError>;

    // ==================== Conversations ====================

    /// Create a new conversation.
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError>;

    /// Update conversation last activity.
    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError>;

    /// Add a message to a conversation.
    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError>;

    /// Ensure a conversation row exists (upsert).
    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError>;

    /// List conversations with a title preview.
    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;

    /// Get or create the singleton assistant conversation.
    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError>;

    /// Create a conversation with specific metadata.
    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError>;

    /// Load messages with cursor-based pagination.
    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError>;

    /// Merge a single key into conversation metadata.
    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;

    /// Read conversation metadata.
    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;

    /// Load all messages for a conversation.
    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError>;

    /// Check if a conversation belongs to a specific user.
    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;

    // ==================== Jobs ====================

    /// Save a job context.
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError>;

    /// Get a job by ID.
    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError>;

    /// Update job status.
    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError>;

    /// Mark job as stuck.
    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError>;

    /// Get stuck jobs.
    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError>;

    // ==================== Actions ====================

    /// Save a job action.
    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError>;

    /// Get actions for a job.
    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError>;

    // ==================== LLM Calls ====================

    /// Record an LLM call.
    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError>;

    // ==================== Estimation Snapshots ====================

    /// Save an estimation snapshot.
    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError>;

    /// Update estimation snapshot with actual values.
    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError>;

    // ==================== Sandbox Jobs ====================

    /// Insert a new sandbox job.
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError>;

    /// Get a sandbox job by ID.
    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError>;

    /// List all sandbox jobs, most recent first.
    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError>;

    /// Update sandbox job status.
    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError>;

    /// Mark stale sandbox jobs as interrupted.
    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError>;

    /// Get sandbox job summary.
    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError>;

    /// List sandbox jobs for a specific user, most recent first.
    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError>;

    /// Get sandbox job summary for a specific user.
    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError>;

    /// Check if a sandbox job belongs to a specific user.
    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;

    /// Update sandbox job mode.
    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError>;

    /// Get sandbox job mode.
    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError>;

    // ==================== Job Events ====================

    /// Persist a job event.
    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError>;

    /// Load job events, returning the most recent `limit` entries (or all if `None`).
    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError>;

    // ==================== Routines ====================

    /// Create a new routine.
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;

    /// Get a routine by ID.
    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError>;

    /// Get a routine by user_id and name.
    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError>;

    /// List routines for a user.
    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError>;

    /// List all enabled event routines.
    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError>;

    /// List due cron routines.
    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError>;

    /// Update a routine.
    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;

    /// Update runtime state after a routine fires.
    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError>;

    /// Delete a routine.
    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError>;

    // ==================== Routine Runs ====================

    /// Record a routine run starting.
    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError>;

    /// Complete a routine run.
    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError>;

    /// List recent runs for a routine.
    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError>;

    /// Count currently running runs for a routine.
    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError>;

    // ==================== Tool Failures ====================

    /// Record a tool failure (upsert).
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError>;

    /// Get broken tools exceeding threshold.
    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError>;

    /// Mark a tool as repaired.
    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError>;

    /// Increment repair attempts.
    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError>;

    // ==================== Settings ====================

    /// Get a single setting.
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;

    /// Get a single setting with metadata.
    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError>;

    /// Set a single setting (upsert).
    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;

    /// Delete a single setting.
    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError>;

    /// List all settings for a user.
    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError>;

    /// Get all settings as a flat map.
    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError>;

    /// Bulk-write settings atomically.
    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError>;

    /// Check if settings exist for a user.
    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError>;

    // ==================== Workspace: Documents ====================

    /// Get a document by path.
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;

    /// Get a document by ID.
    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError>;

    /// Get or create a document by path.
    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;

    /// Update a document's content.
    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError>;

    /// Delete a document by path.
    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError>;

    /// List files and directories in a directory path.
    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError>;

    /// List all file paths in the workspace.
    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError>;

    /// List all documents for a user.
    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError>;

    // ==================== Workspace: Chunks ====================

    /// Delete all chunks for a document.
    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError>;

    /// Insert a chunk.
    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError>;

    /// Update a chunk's embedding.
    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;

    /// Get chunks without embeddings for backfilling.
    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError>;

    // ==================== Workspace: Search ====================

    /// Perform hybrid search combining FTS and vector similarity.
    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError>;
}
