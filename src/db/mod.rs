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
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
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
    ) -> Result<(), DatabaseError>;
    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
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
    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError>;
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
