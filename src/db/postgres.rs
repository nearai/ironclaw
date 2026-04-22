//! PostgreSQL backend for the Database trait.
//!
//! Delegates to the existing `Store` (history) and `Repository` (workspace)
//! implementations, avoiding SQL duplication.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::agent::BrokenTool;
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::config::DatabaseConfig;
use crate::context::{ActionRecord, JobContext, JobState};
use crate::db::{
    ApiTokenRecord, BudgetStore, ChannelPairingStore, ConversationStore, Database, IdentityStore,
    JobStore, PairingRequestRecord, RoutineStore, SandboxStore, SettingsStore, ToolFailureStore,
    UserIdentityRecord, UserRecord, UserStore, WorkspaceStore,
};
use crate::error::{DatabaseError, WorkspaceError};
use crate::history::{
    AgentJobRecord, AgentJobSummary, ConversationMessage, ConversationSummary, JobEventRecord,
    LlmCallRecord, SandboxJobRecord, SandboxJobSummary, SettingRow, Store,
};
use crate::workspace::{
    ChunkWrite, DocumentVersion, MemoryChunk, MemoryDocument, Repository, SearchConfig,
    SearchResult, VersionSummary, WorkspaceEntry,
};

/// PostgreSQL database backend.
///
/// Wraps the existing `Store` (for history/conversations/jobs/routines/settings)
/// and `Repository` (for workspace documents/chunks/search) to implement the
/// unified `Database` trait.
pub struct PgBackend {
    store: Store,
    repo: Repository,
}

impl PgBackend {
    /// Create a new PostgreSQL backend from configuration.
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let store = Store::new(config).await?;
        let repo = Repository::new(store.pool());
        Ok(Self { store, repo })
    }

    /// Get a clone of the connection pool.
    ///
    /// Useful for sharing with components that still need raw pool access.
    pub fn pool(&self) -> Pool {
        self.store.pool()
    }
}

// ==================== Database (supertrait) ====================

#[async_trait]
impl Database for PgBackend {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        self.store.run_migrations().await
    }

    async fn migrate_default_owner(&self, owner_id: &str) -> Result<(), DatabaseError> {
        let mut client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        // Only tables with a real `user_id` column participate in the legacy
        // 'default' -> owner rewrite. `dynamic_tools` is intentionally excluded:
        // it is ownerless today and scoped by `scope`, not `user_id`.
        let tables = [
            "conversations",
            "memory_documents",
            "heartbeat_state",
            "secrets",
            "wasm_tools",
            "routines",
            "settings",
            "agent_jobs",
            "api_tokens",
        ];
        for table in &tables {
            tx.execute(
                &format!(
                    "UPDATE {} SET user_id = $1 WHERE user_id = 'default'",
                    table
                ),
                &[&owner_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("migrate_default_owner {table}: {e}")))?;
        }
        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }
}

// ==================== ConversationStore ====================

#[async_trait]
impl ConversationStore for PgBackend {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation(channel, user_id, thread_id)
            .await
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.store.touch_conversation(id).await
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .add_conversation_message(conversation_id, role, content)
            .await
    }

    async fn add_conversation_message_if_empty(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.store.pool().get().await?;
        let id = Uuid::new_v4();
        let result = conn
            .execute(
                "INSERT INTO conversation_messages (id, conversation_id, role, content) \
                 SELECT $1, $2, $3, $4 \
                 WHERE NOT EXISTS ( \
                     SELECT 1 FROM conversation_messages WHERE conversation_id = $2 \
                 )",
                &[&id, &conversation_id, &role, &content],
            )
            .await?;
        if result > 0 {
            self.store.touch_conversation(conversation_id).await?;
        }
        Ok(result > 0)
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
        source_channel: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        self.store
            .ensure_conversation(id, channel, user_id, thread_id, source_channel)
            .await
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_conversations_with_preview(user_id, channel, limit)
            .await
    }

    async fn list_conversations_all_channels(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_conversations_all_channels(user_id, limit)
            .await
    }

    async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_routine_conversation(routine_id, routine_name, user_id)
            .await
    }

    async fn find_routine_conversation(
        &self,
        routine_id: Uuid,
        user_id: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        self.store
            .find_routine_conversation(routine_id, user_id)
            .await
    }

    async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_heartbeat_conversation(user_id)
            .await
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_assistant_conversation(user_id, channel)
            .await
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation_with_metadata(channel, user_id, metadata)
            .await
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        self.store
            .list_conversation_messages_paginated(conversation_id, before, limit)
            .await
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_conversation_metadata_field(id, key, value)
            .await
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.store.get_conversation_metadata(id).await
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        self.store.list_conversation_messages(conversation_id).await
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .conversation_belongs_to_user(conversation_id, user_id)
            .await
    }

    async fn get_conversation_source_channel(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        self.store
            .get_conversation_source_channel(conversation_id)
            .await
    }
}

// ==================== JobStore ====================

#[async_trait]
impl JobStore for PgBackend {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        self.store.save_job(ctx).await
    }

    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        self.store.get_job(id).await
    }

    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_job_status(id, status, failure_reason)
            .await
    }

    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.store.mark_job_stuck(id).await
    }

    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        self.store.get_stuck_jobs().await
    }

    async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        self.store.list_agent_jobs().await
    }

    async fn list_agent_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        self.store.list_agent_jobs_for_user(user_id).await
    }

    async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError> {
        self.store.agent_job_summary().await
    }

    async fn agent_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<AgentJobSummary, DatabaseError> {
        self.store.agent_job_summary_for_user(user_id).await
    }

    async fn get_agent_job_failure_reason(
        &self,
        id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        self.store.get_agent_job_failure_reason(id).await
    }

    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError> {
        self.store.save_action(job_id, action).await
    }

    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        self.store.get_job_actions(job_id).await
    }

    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        self.store.record_llm_call(record).await
    }

    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .save_estimation_snapshot(
                job_id,
                category,
                tool_names,
                estimated_cost,
                estimated_time_secs,
                estimated_value,
            )
            .await
    }

    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_estimation_actuals(id, actual_cost, actual_time_secs, actual_value)
            .await
    }

    async fn create_system_job(&self, user_id: &str, source: &str) -> Result<Uuid, DatabaseError> {
        self.store.create_system_job(user_id, source).await
    }
}

// ==================== SandboxStore ====================

#[async_trait]
impl SandboxStore for PgBackend {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        self.store.save_sandbox_job(job).await
    }

    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        self.store.get_sandbox_job(id).await
    }

    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store.list_sandbox_jobs().await
    }

    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_sandbox_job_status(id, status, success, message, started_at, completed_at)
            .await
    }

    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        self.store.cleanup_stale_sandbox_jobs().await
    }

    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary().await
    }

    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store.list_sandbox_jobs_for_user(user_id).await
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary_for_user(user_id).await
    }

    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .sandbox_job_belongs_to_user(job_id, user_id)
            .await
    }

    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        self.store.update_sandbox_job_mode(id, mode).await
    }

    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        self.store.get_sandbox_job_mode(id).await
    }

    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store.save_job_event(job_id, event_type, data).await
    }

    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        self.store.list_job_events(job_id, limit).await
    }
}

// ==================== RoutineStore ====================

#[async_trait]
impl RoutineStore for PgBackend {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.store.create_routine(routine).await
    }

    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        self.store.get_routine(id).await
    }

    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.store.get_routine_by_name(user_id, name).await
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_routines(user_id).await
    }

    async fn list_all_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_all_routines().await
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_event_routines().await
    }

    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_due_cron_routines().await
    }

    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.store.update_routine(routine).await
    }

    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_routine_runtime(
                id,
                last_run_at,
                next_fire_at,
                run_count,
                consecutive_failures,
                state,
            )
            .await
    }

    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_routine(id).await
    }

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        self.store.create_routine_run(run).await
    }

    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        self.store
            .complete_routine_run(id, status, result_summary, tokens_used)
            .await
    }

    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        self.store.list_routine_runs(routine_id, limit).await
    }

    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        self.store.count_running_routine_runs(routine_id).await
    }

    async fn count_running_routine_runs_batch(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, i64>, DatabaseError> {
        self.store
            .count_running_routine_runs_batch(routine_ids)
            .await
    }

    async fn batch_get_last_run_status(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<std::collections::HashMap<Uuid, crate::agent::routine::RunStatus>, DatabaseError>
    {
        self.store.batch_get_last_run_status(routine_ids).await
    }

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.store.link_routine_run_to_job(run_id, job_id).await
    }

    async fn get_webhook_routine_by_path(
        &self,
        path: &str,
        user_id: Option<&str>,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.store.get_webhook_routine_by_path(path, user_id).await
    }

    async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError> {
        self.store.list_dispatched_routine_runs().await
    }
}

// ==================== ToolFailureStore ====================

#[async_trait]
impl ToolFailureStore for PgBackend {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        self.store
            .record_tool_failure(tool_name, error_message)
            .await
    }

    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        self.store.get_broken_tools(threshold).await
    }

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.mark_tool_repaired(tool_name).await
    }

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.increment_repair_attempts(tool_name).await
    }
}

// ==================== SettingsStore ====================

#[async_trait]
impl SettingsStore for PgBackend {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.store.get_setting(user_id, key).await
    }

    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        self.store.get_setting_full(user_id, key).await
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store.set_setting(user_id, key, value).await
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        self.store.delete_setting(user_id, key).await
    }

    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        self.store.list_settings(user_id).await
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.store.get_all_settings(user_id).await
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.store.set_all_settings(user_id, settings).await
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        self.store.has_settings(user_id).await
    }
}

// ==================== WorkspaceStore ====================

#[async_trait]
impl WorkspaceStore for PgBackend {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        self.repo
            .get_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        self.repo.get_document_by_id(id).await
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        self.repo
            .get_or_create_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        self.repo.update_document(id, content).await
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        self.repo
            .delete_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        self.repo.list_directory(user_id, agent_id, directory).await
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        self.repo.list_all_paths(user_id, agent_id).await
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        self.repo.list_documents(user_id, agent_id).await
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        self.repo.delete_chunks(document_id).await
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        self.repo
            .insert_chunk(document_id, chunk_index, content, embedding)
            .await
    }

    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[ChunkWrite],
    ) -> Result<(), WorkspaceError> {
        self.repo.replace_chunks(document_id, chunks).await
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        self.repo.update_chunk_embedding(chunk_id, embedding).await
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        self.repo
            .get_chunks_without_embeddings(user_id, agent_id, limit)
            .await
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        self.repo
            .hybrid_search(user_id, agent_id, query, embedding, config)
            .await
    }

    // Optimized multi-scope overrides using `ANY($1::text[])` SQL.

    async fn hybrid_search_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        self.repo
            .hybrid_search_multi(user_ids, agent_id, query, embedding, config)
            .await
    }

    async fn list_all_paths_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        self.repo.list_all_paths_multi(user_ids, agent_id).await
    }

    async fn get_document_by_path_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        self.repo
            .get_document_by_path_multi(user_ids, agent_id, path)
            .await
    }

    async fn list_directory_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        self.repo
            .list_directory_multi(user_ids, agent_id, directory)
            .await
    }

    // ==================== Metadata ====================

    async fn update_document_metadata(
        &self,
        id: Uuid,
        metadata: &serde_json::Value,
    ) -> Result<(), WorkspaceError> {
        self.repo.update_document_metadata(id, metadata).await
    }

    async fn find_config_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        self.repo.find_config_documents(user_id, agent_id).await
    }

    // ==================== Versioning ====================

    async fn save_version(
        &self,
        document_id: Uuid,
        content: &str,
        content_hash: &str,
        changed_by: Option<&str>,
    ) -> Result<i32, WorkspaceError> {
        self.repo
            .save_version(document_id, content, content_hash, changed_by)
            .await
    }

    async fn get_version(
        &self,
        document_id: Uuid,
        version: i32,
    ) -> Result<DocumentVersion, WorkspaceError> {
        self.repo.get_version(document_id, version).await
    }

    async fn list_versions(
        &self,
        document_id: Uuid,
        limit: i64,
    ) -> Result<Vec<VersionSummary>, WorkspaceError> {
        self.repo.list_versions(document_id, limit).await
    }

    async fn get_latest_version_number(
        &self,
        document_id: Uuid,
    ) -> Result<Option<i32>, WorkspaceError> {
        self.repo.get_latest_version_number(document_id).await
    }

    async fn prune_versions(
        &self,
        document_id: Uuid,
        keep_count: i32,
    ) -> Result<u64, WorkspaceError> {
        self.repo.prune_versions(document_id, keep_count).await
    }
}

// ==================== UserStore ====================

#[async_trait]
impl UserStore for PgBackend {
    async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError> {
        self.store.create_user(user).await
    }

    async fn get_or_create_user(&self, user: UserRecord) -> Result<(), DatabaseError> {
        let mut client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let tx = client.transaction().await?;
        let rows = tx
            .execute(
                "INSERT INTO users (id, email, display_name, status, role, created_at, updated_at, last_login_at, created_by, metadata)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT (id) DO NOTHING",
                &[
                    &user.id,
                    &user.email,
                    &user.display_name,
                    &user.status,
                    &user.role,
                    &user.created_at,
                    &user.updated_at,
                    &user.last_login_at,
                    &user.created_by,
                    &user.metadata,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_or_create_user: {e}")))?;
        if rows > 0 {
            Store::seed_initial_assistant_thread(&tx, &user.id, user.created_at).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    async fn get_user(&self, id: &str) -> Result<Option<UserRecord>, DatabaseError> {
        self.store.get_user(id).await
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DatabaseError> {
        self.store.get_user_by_email(email).await
    }

    async fn list_users(&self, status: Option<&str>) -> Result<Vec<UserRecord>, DatabaseError> {
        self.store.list_users(status).await
    }

    async fn update_user_status(&self, id: &str, status: &str) -> Result<(), DatabaseError> {
        self.store.update_user_status(id, status).await
    }

    async fn update_user_role(&self, id: &str, role: &str) -> Result<(), DatabaseError> {
        self.store.update_user_role(id, role).await
    }

    async fn update_user_profile(
        &self,
        id: &str,
        display_name: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_user_profile(id, display_name, metadata)
            .await
    }

    async fn record_login(&self, id: &str) -> Result<(), DatabaseError> {
        self.store.record_login(id).await
    }

    async fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError> {
        self.store
            .create_api_token(user_id, name, token_hash, token_prefix, expires_at)
            .await
    }

    async fn list_api_tokens(&self, user_id: &str) -> Result<Vec<ApiTokenRecord>, DatabaseError> {
        self.store.list_api_tokens(user_id).await
    }

    async fn revoke_api_token(&self, token_id: Uuid, user_id: &str) -> Result<bool, DatabaseError> {
        self.store.revoke_api_token(token_id, user_id).await
    }

    async fn authenticate_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<(ApiTokenRecord, UserRecord)>, DatabaseError> {
        self.store.authenticate_token(token_hash).await
    }

    async fn record_token_usage(&self, token_id: Uuid) -> Result<(), DatabaseError> {
        self.store.record_token_usage(token_id).await
    }

    async fn has_any_users(&self) -> Result<bool, DatabaseError> {
        self.store.has_any_users().await
    }

    async fn delete_user(&self, id: &str) -> Result<bool, DatabaseError> {
        self.store.delete_user(id).await
    }

    async fn user_usage_stats(
        &self,
        user_id: Option<&str>,
        since: DateTime<Utc>,
    ) -> Result<Vec<crate::db::UserUsageStats>, DatabaseError> {
        self.store.user_usage_stats(user_id, since).await
    }

    async fn user_summary_stats(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<crate::db::UserSummaryStats>, DatabaseError> {
        self.store.user_summary_stats(user_id).await
    }

    async fn admin_usage_summary(
        &self,
        since: DateTime<Utc>,
    ) -> Result<crate::db::AdminUsageSummary, DatabaseError> {
        self.store.admin_usage_summary(since).await
    }

    async fn create_user_with_token(
        &self,
        user: &UserRecord,
        token_name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError> {
        self.store
            .create_user_with_token(user, token_name, token_hash, token_prefix, expires_at)
            .await
    }
}

// ==================== ChannelPairingStore ====================

#[async_trait]
impl ChannelPairingStore for PgBackend {
    async fn resolve_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<Option<crate::ownership::UserId>, DatabaseError> {
        use crate::ownership::{UserId, UserRole};
        let channel = crate::pairing::normalize_channel_name(channel);
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let row = client
            .query_opt(
                "SELECT ci.owner_id, u.role
                 FROM channel_identities ci
                 JOIN users u ON u.id = ci.owner_id
                 WHERE ci.channel = $1 AND ci.external_id = $2
                   AND u.status = 'active'",
                &[&channel, &external_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(row.map(|r| {
            let owner_id: String = r.get(0);
            let role_str: String = r.get(1);
            let role = UserRole::from_db_role(&role_str);
            UserId::from_trusted(owner_id, role)
        }))
    }

    async fn read_allow_from(&self, channel: &str) -> Result<Vec<String>, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let rows = client
            .query(
                "SELECT ci.external_id
                 FROM channel_identities ci
                 JOIN users u ON u.id = ci.owner_id
                 WHERE ci.channel = $1
                   AND u.status = 'active'
                 ORDER BY ci.external_id ASC",
                &[&channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows.into_iter().map(|row| row.get(0)).collect())
    }

    async fn resolve_channel_external_id_for_owner(
        &self,
        channel: &str,
        owner_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let row = client
            .query_opt(
                "SELECT ci.external_id
                 FROM channel_identities ci
                 LEFT JOIN users u ON u.id = ci.owner_id
                 WHERE ci.channel = $1
                   AND ci.owner_id = $2
                   AND (u.id IS NULL OR u.status = 'active')
                 ORDER BY ci.external_id ASC
                 LIMIT 1",
                &[&channel, &owner_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(row.map(|r| r.get(0)))
    }

    async fn upsert_pairing_request(
        &self,
        channel: &str,
        external_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<PairingRequestRecord, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let mut client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // Serialize upserts for the same normalized sender key so PostgreSQL
        // preserves the single-live-code guarantee that libSQL gets from
        // BEGIN IMMEDIATE.
        let lock_key = format!(
            "{}:{}:{}:{}",
            channel.len(),
            channel,
            external_id.len(),
            external_id
        );
        tx.query(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&lock_key],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        tx.execute(
            "UPDATE pairing_requests
             SET expires_at = NOW()
             WHERE channel = $1 AND external_id = $2
               AND approved_at IS NULL AND expires_at > NOW()",
            &[&channel, &external_id],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(15);
        let meta_json: Option<serde_json::Value> = meta;

        // Retry loop: regenerate code on UNIQUE violation (code collision)
        for attempt in 0..3 {
            let code = crate::db::generate_pairing_code();
            match tx
                .query_one(
                    "INSERT INTO pairing_requests (id, channel, external_id, code, meta, expires_at)
                     VALUES (gen_random_uuid(), $1, $2, $3, $4, $5)
                     RETURNING id, channel, external_id, code, created_at, expires_at",
                    &[&channel, &external_id, &code, &meta_json, &expires_at],
                )
                .await
            {
                Ok(row) => {
                    tx.commit()
                        .await
                        .map_err(|e| DatabaseError::Query(e.to_string()))?;
                    return Ok(PairingRequestRecord {
                        id: row.get(0),
                        channel: row.get(1),
                        external_id: row.get(2),
                        code: row.get(3),
                        created: true,
                        created_at: row.get(4),
                        expires_at: row.get(5),
                    });
                }
                Err(e) => {
                    let is_unique = e
                        .code()
                        .is_some_and(|c| *c == tokio_postgres::error::SqlState::UNIQUE_VIOLATION);
                    if attempt < 2 && is_unique {
                        continue;
                    }
                    return Err(DatabaseError::Query(e.to_string()));
                }
            }
        }

        Err(DatabaseError::Query(
            "failed to generate unique pairing code after 3 attempts".to_string(),
        ))
    }

    async fn approve_pairing(
        &self,
        channel: &str,
        code: &str,
        owner_id: &str,
    ) -> Result<crate::db::PairingApprovalRecord, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let mut client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let row = tx
            .query_opt(
                "SELECT id, channel, external_id FROM pairing_requests
                 WHERE UPPER(code) = UPPER($1)
                   AND channel = $2
                   AND approved_at IS NULL
                   AND expires_at > NOW()
                 FOR UPDATE",
                &[&code, &channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "pairing_request".to_string(),
                id: code.to_string(),
            })?;

        let req_id: uuid::Uuid = row.get(0);
        let channel: String = row.get(1);
        let external_id: String = row.get(2);
        let previous_owner_id = tx
            .query_opt(
                "SELECT owner_id
                 FROM channel_identities
                 WHERE channel = $1 AND external_id = $2
                 FOR UPDATE",
                &[&channel, &external_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .map(|row| row.get(0));

        tx.execute(
            "UPDATE pairing_requests SET owner_id = $1, approved_at = NOW() WHERE id = $2",
            &[&owner_id, &req_id],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        tx.execute(
            "INSERT INTO channel_identities (id, owner_id, channel, external_id)
             VALUES (gen_random_uuid(), $1, $2, $3)
             ON CONFLICT (channel, external_id) DO UPDATE SET owner_id = $1",
            &[&owner_id, &channel, &external_id],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(crate::db::PairingApprovalRecord {
            request_id: req_id,
            channel,
            external_id,
            owner_id: owner_id.to_string(),
            previous_owner_id,
        })
    }

    async fn revert_pairing_approval(
        &self,
        approval: &crate::db::PairingApprovalRecord,
    ) -> Result<(), DatabaseError> {
        let mut client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let updated = tx
            .execute(
                "UPDATE pairing_requests
             SET owner_id = NULL, approved_at = NULL
             WHERE id = $1 AND owner_id = $2 AND approved_at IS NOT NULL",
                &[&approval.request_id, &approval.owner_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if updated == 0 {
            return Err(DatabaseError::NotFound {
                entity: "pairing_approval".to_string(),
                id: approval.request_id.to_string(),
            });
        }

        if let Some(previous_owner_id) = approval.previous_owner_id.as_ref() {
            tx.execute(
                "INSERT INTO channel_identities (id, owner_id, channel, external_id)
                 VALUES (gen_random_uuid(), $1, $2, $3)
                 ON CONFLICT (channel, external_id) DO UPDATE SET owner_id = $1",
                &[previous_owner_id, &approval.channel, &approval.external_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        } else {
            tx.execute(
                "DELETE FROM channel_identities
                 WHERE channel = $1 AND external_id = $2 AND owner_id = $3",
                &[&approval.channel, &approval.external_id, &approval.owner_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))
    }

    async fn list_pending_pairings(
        &self,
        channel: &str,
    ) -> Result<Vec<PairingRequestRecord>, DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        let rows = client
            .query(
                "SELECT id, channel, external_id, code, created_at, expires_at
                 FROM pairing_requests
                 WHERE channel = $1 AND approved_at IS NULL AND expires_at > NOW()
                 ORDER BY created_at ASC",
                &[&channel],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows
            .iter()
            .map(|r| PairingRequestRecord {
                id: r.get(0),
                channel: r.get(1),
                external_id: r.get(2),
                code: r.get(3),
                created: false,
                created_at: r.get(4),
                expires_at: r.get(5),
            })
            .collect())
    }

    async fn remove_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<(), DatabaseError> {
        let channel = crate::pairing::normalize_channel_name(channel);
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(e.to_string()))?;
        client
            .execute(
                "DELETE FROM channel_identities WHERE channel = $1 AND external_id = $2",
                &[&channel, &external_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }
}

// ==================== IdentityStore ====================

fn row_to_identity(row: &tokio_postgres::Row) -> UserIdentityRecord {
    UserIdentityRecord {
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
    }
}

#[async_trait]
impl IdentityStore for PgBackend {
    async fn get_identity_by_provider(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        let conn = self.store.pool().get().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, provider, provider_user_id, email, email_verified, \
                 display_name, avatar_url, raw_profile, created_at, updated_at \
                 FROM user_identities WHERE provider = $1 AND provider_user_id = $2",
                &[&provider, &provider_user_id],
            )
            .await?;
        Ok(row.as_ref().map(row_to_identity))
    }

    async fn list_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserIdentityRecord>, DatabaseError> {
        let conn = self.store.pool().get().await?;
        let rows = conn
            .query(
                "SELECT id, user_id, provider, provider_user_id, email, email_verified, \
                 display_name, avatar_url, raw_profile, created_at, updated_at \
                 FROM user_identities WHERE user_id = $1 ORDER BY created_at",
                &[&user_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_identity).collect())
    }

    async fn create_identity(&self, identity: &UserIdentityRecord) -> Result<(), DatabaseError> {
        let conn = self.store.pool().get().await?;
        conn.execute(
            "INSERT INTO user_identities \
             (id, user_id, provider, provider_user_id, email, email_verified, \
              display_name, avatar_url, raw_profile, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            &[
                &identity.id,
                &identity.user_id,
                &identity.provider,
                &identity.provider_user_id,
                &identity.email,
                &identity.email_verified,
                &identity.display_name,
                &identity.avatar_url,
                &identity.raw_profile,
                &identity.created_at,
                &identity.updated_at,
            ],
        )
        .await?;
        Ok(())
    }

    async fn update_identity_profile(
        &self,
        provider: &str,
        provider_user_id: &str,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.store.pool().get().await?;
        conn.execute(
            "UPDATE user_identities SET display_name = COALESCE($3, display_name), \
             avatar_url = COALESCE($4, avatar_url), updated_at = NOW() \
             WHERE provider = $1 AND provider_user_id = $2",
            &[&provider, &provider_user_id, &display_name, &avatar_url],
        )
        .await?;
        Ok(())
    }

    async fn find_identity_by_verified_email(
        &self,
        email: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        let conn = self.store.pool().get().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, provider, provider_user_id, email, email_verified, \
                 display_name, avatar_url, raw_profile, created_at, updated_at \
                 FROM user_identities WHERE LOWER(email) = LOWER($1) AND email_verified = true LIMIT 1",
                &[&email],
            )
            .await?;
        Ok(row.as_ref().map(row_to_identity))
    }

    async fn create_user_with_identity(
        &self,
        user: &UserRecord,
        identity: &UserIdentityRecord,
    ) -> Result<(), DatabaseError> {
        let mut conn = self.store.pool().get().await?;
        let tx = conn.transaction().await?;

        tx.execute(
            "INSERT INTO users (id, email, display_name, status, role, created_at, \
             updated_at, last_login_at, created_by, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            &[
                &user.id,
                &user.email,
                &user.display_name,
                &user.status,
                &user.role,
                &user.created_at,
                &user.updated_at,
                &user.last_login_at,
                &user.created_by,
                &user.metadata,
            ],
        )
        .await?;

        tx.execute(
            "INSERT INTO user_identities \
             (id, user_id, provider, provider_user_id, email, email_verified, \
              display_name, avatar_url, raw_profile, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            &[
                &identity.id,
                &identity.user_id,
                &identity.provider,
                &identity.provider_user_id,
                &identity.email,
                &identity.email_verified,
                &identity.display_name,
                &identity.avatar_url,
                &identity.raw_profile,
                &identity.created_at,
                &identity.updated_at,
            ],
        )
        .await?;

        // Atomically promote to admin if this is the only user in the table.
        // Under READ COMMITTED, two concurrent transactions could both see
        // COUNT(*)=1 (each sees its own uncommitted insert). Use an advisory
        // lock to serialize the first-user election across transactions.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext('first_user_admin_election'))",
            &[],
        )
        .await?;
        tx.execute(
            "UPDATE users SET role = 'admin' \
             WHERE id = $1 AND (SELECT COUNT(*) FROM users) = 1",
            &[&user.id],
        )
        .await?;

        Store::seed_initial_assistant_thread(&tx, &user.id, user.created_at).await?;

        tx.commit().await?;
        Ok(())
    }
}

// ==================== BudgetStore (issue #2843) ====================

#[async_trait]
impl BudgetStore for PgBackend {
    async fn save_budget(
        &self,
        budget: &ironclaw_engine::types::budget::Budget,
    ) -> Result<(), DatabaseError> {
        let client = self.pool().get().await?;
        let usd = budget.limit.usd;
        let tokens: Option<i64> = budget.limit.tokens.map(|n| n as i64);
        let wall_clock: Option<i64> = budget.limit.wall_clock_secs.map(|n| n as i64);
        let (period_tz, period_unit) = match &budget.period {
            ironclaw_engine::types::budget::BudgetPeriod::Calendar { tz, unit } => {
                (Some(tz.clone()), Some(unit.as_str().to_string()))
            }
            _ => (None, None),
        };
        client
            .execute(
                "INSERT INTO budgets (
                    id, user_id, scope_kind, scope_id, limit_usd, limit_tokens,
                    limit_wall_clock_secs, period_kind, period_tz, period_unit,
                    source, active, created_at, created_by
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14
                )",
                &[
                    &budget.id.0,
                    &budget.scope.user_id(),
                    &budget.scope.kind_str(),
                    &budget.scope.scope_id(),
                    &usd,
                    &tokens,
                    &wall_clock,
                    &budget.period.kind_str(),
                    &period_tz,
                    &period_unit,
                    &budget.source.as_str(),
                    &budget.active,
                    &budget.created_at,
                    &budget.created_by,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("save_budget: {e}")))?;
        Ok(())
    }

    async fn load_budget(
        &self,
        id: ironclaw_engine::types::budget::BudgetId,
    ) -> Result<Option<ironclaw_engine::types::budget::Budget>, DatabaseError> {
        let client = self.pool().get().await?;
        let row = client
            .query_opt(
                "SELECT id, user_id, scope_kind, scope_id, limit_usd, limit_tokens,
                    limit_wall_clock_secs, period_kind, period_tz, period_unit,
                    source, active, created_at, created_by
                 FROM budgets WHERE id = $1",
                &[&id.0],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("load_budget: {e}")))?;
        row.map(budget_from_row).transpose()
    }

    async fn list_active_budgets_for_scope(
        &self,
        scope_kind: &str,
        scope_id: &str,
    ) -> Result<Vec<ironclaw_engine::types::budget::Budget>, DatabaseError> {
        let client = self.pool().get().await?;
        let rows = client
            .query(
                "SELECT id, user_id, scope_kind, scope_id, limit_usd, limit_tokens,
                    limit_wall_clock_secs, period_kind, period_tz, period_unit,
                    source, active, created_at, created_by
                 FROM budgets
                 WHERE scope_kind = $1 AND scope_id = $2 AND active = TRUE",
                &[&scope_kind, &scope_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("list_active_budgets_for_scope: {e}")))?;
        rows.into_iter().map(budget_from_row).collect()
    }

    async fn deactivate_budget(
        &self,
        id: ironclaw_engine::types::budget::BudgetId,
    ) -> Result<(), DatabaseError> {
        let client = self.pool().get().await?;
        client
            .execute("UPDATE budgets SET active = FALSE WHERE id = $1", &[&id.0])
            .await
            .map_err(|e| DatabaseError::Query(format!("deactivate_budget: {e}")))?;
        Ok(())
    }

    async fn get_or_create_ledger_for_period(
        &self,
        budget_id: ironclaw_engine::types::budget::BudgetId,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<ironclaw_engine::types::budget::BudgetLedger, DatabaseError> {
        let client = self.pool().get().await?;
        // ON CONFLICT DO NOTHING so concurrent callers don't race.
        client
            .execute(
                "INSERT INTO budget_ledgers (
                    budget_id, period_start, period_end, spent_usd, reserved_usd,
                    tokens_used, updated_at
                 ) VALUES ($1, $2, $3, 0, 0, 0, $4)
                 ON CONFLICT (budget_id, period_start) DO NOTHING",
                &[&budget_id.0, &period_start, &period_end, &now],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_or_create_ledger insert: {e}")))?;
        let row = client
            .query_one(
                "SELECT budget_id, period_start, period_end, spent_usd, reserved_usd,
                    tokens_used, updated_at
                 FROM budget_ledgers
                 WHERE budget_id = $1 AND period_start = $2",
                &[&budget_id.0, &period_start],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_or_create_ledger select: {e}")))?;
        ledger_from_row(row)
    }

    async fn reserve_atomic(
        &self,
        budget_id: ironclaw_engine::types::budget::BudgetId,
        period_start: DateTime<Utc>,
        period_end: DateTime<Utc>,
        requested_usd: Decimal,
        _requested_tokens: u64,
        limit_usd: Decimal,
        now: DateTime<Utc>,
    ) -> Result<
        Option<(
            ironclaw_engine::types::budget::ReservationId,
            ironclaw_engine::types::budget::BudgetLedger,
        )>,
        DatabaseError,
    > {
        if requested_usd.is_sign_negative() {
            return Err(DatabaseError::Query(
                "reserve_atomic: requested_usd must be non-negative".into(),
            ));
        }

        let client = self.pool().get().await?;

        // Ensure the ledger row exists first — ON CONFLICT DO NOTHING.
        client
            .execute(
                "INSERT INTO budget_ledgers (
                    budget_id, period_start, period_end, spent_usd, reserved_usd,
                    tokens_used, updated_at
                 ) VALUES ($1, $2, $3, 0, 0, 0, $4)
                 ON CONFLICT (budget_id, period_start) DO NOTHING",
                &[&budget_id.0, &period_start, &period_end, &now],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("reserve_atomic seed ledger: {e}")))?;

        // Conditional UPDATE: the row is modified ONLY if the check
        // passes. Empty result ⇒ clean denial. `RETURNING` gives back
        // the new ledger state in the same round-trip.
        //
        // NUMERIC arithmetic in PostgreSQL is lossless over the
        // rust_decimal precision we use, so the in-SQL comparison is
        // safe — no re-parse in application code, no TOCTOU window.
        let row = client
            .query_opt(
                "UPDATE budget_ledgers
                    SET reserved_usd = reserved_usd + $3,
                        updated_at = $4
                  WHERE budget_id = $1
                    AND period_start = $2
                    AND spent_usd + reserved_usd + $3 <= $5
                 RETURNING budget_id, period_start, period_end, spent_usd,
                           reserved_usd, tokens_used, updated_at",
                &[
                    &budget_id.0,
                    &period_start,
                    &requested_usd,
                    &now,
                    &limit_usd,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("reserve_atomic UPDATE: {e}")))?;

        let Some(row) = row else {
            return Ok(None);
        };
        let ledger = ledger_from_row(row)?;
        Ok(Some((
            ironclaw_engine::types::budget::ReservationId::new(),
            ledger,
        )))
    }

    async fn reconcile_reservation(
        &self,
        _reservation_id: ironclaw_engine::types::budget::ReservationId,
        budget_id: ironclaw_engine::types::budget::BudgetId,
        period_start: DateTime<Utc>,
        original_reserved_usd: Decimal,
        actual_usd: Decimal,
        actual_tokens: u64,
        now: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        if actual_usd.is_sign_negative() || original_reserved_usd.is_sign_negative() {
            return Err(DatabaseError::Query(
                "reconcile_reservation: amounts must be non-negative".into(),
            ));
        }
        let tokens_delta = actual_tokens as i64;
        let client = self.pool().get().await?;
        client
            .execute(
                "UPDATE budget_ledgers
                    SET spent_usd = spent_usd + $3,
                        reserved_usd = GREATEST(reserved_usd - $4, 0),
                        tokens_used = tokens_used + $5,
                        updated_at = $6
                  WHERE budget_id = $1 AND period_start = $2",
                &[
                    &budget_id.0,
                    &period_start,
                    &actual_usd,
                    &original_reserved_usd,
                    &tokens_delta,
                    &now,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("reconcile_reservation: {e}")))?;
        Ok(())
    }

    async fn release_reservation(
        &self,
        reservation_id: ironclaw_engine::types::budget::ReservationId,
        budget_id: ironclaw_engine::types::budget::BudgetId,
        period_start: DateTime<Utc>,
        original_reserved_usd: Decimal,
        now: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        self.reconcile_reservation(
            reservation_id,
            budget_id,
            period_start,
            original_reserved_usd,
            Decimal::ZERO,
            0,
            now,
        )
        .await
    }

    async fn record_budget_event(
        &self,
        id: Uuid,
        budget_id: ironclaw_engine::types::budget::BudgetId,
        thread_id: Option<ironclaw_engine::ThreadId>,
        event_kind: &str,
        amount_usd: Option<Decimal>,
        tokens: Option<u64>,
        reason: Option<&str>,
        actor_user_id: &str,
        created_at: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        let tokens_i64: Option<i64> = tokens.map(|n| n as i64);
        let thread_uuid: Option<Uuid> = thread_id.map(|t| t.0);
        let client = self.pool().get().await?;
        client
            .execute(
                "INSERT INTO budget_events (
                    id, budget_id, thread_id, event_kind, amount_usd, tokens,
                    reason, actor_user_id, created_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
                &[
                    &id,
                    &budget_id.0,
                    &thread_uuid,
                    &event_kind,
                    &amount_usd,
                    &tokens_i64,
                    &reason,
                    &actor_user_id,
                    &created_at,
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("record_budget_event: {e}")))?;
        Ok(())
    }
}

fn budget_from_row(
    row: tokio_postgres::Row,
) -> Result<ironclaw_engine::types::budget::Budget, DatabaseError> {
    use ironclaw_engine::types::budget as b;

    let id: Uuid = row.get("id");
    let user_id: String = row.get("user_id");
    let scope_kind: String = row.get("scope_kind");
    let scope_id: String = row.get("scope_id");

    let scope = rehydrate_scope(&scope_kind, &scope_id, user_id)?;

    let tokens: Option<i64> = row.get("limit_tokens");
    let wall_clock: Option<i64> = row.get("limit_wall_clock_secs");
    let limit = b::BudgetLimit {
        usd: row.get("limit_usd"),
        tokens: tokens.map(|n| n as u64),
        wall_clock_secs: wall_clock.map(|n| n as u64),
    };

    let period_kind: String = row.get("period_kind");
    let period_tz: Option<String> = row.get("period_tz");
    let period_unit: Option<String> = row.get("period_unit");
    let period = rehydrate_period(&period_kind, period_tz, period_unit)?;

    let source_str: String = row.get("source");
    let source = match source_str.as_str() {
        "user_override" => b::BudgetSource::UserOverride,
        "inherited" => b::BudgetSource::InheritedFromParent,
        _ => b::BudgetSource::Default,
    };

    Ok(b::Budget {
        id: b::BudgetId(id),
        scope,
        limit,
        period,
        source,
        active: row.get("active"),
        created_at: row.get("created_at"),
        created_by: row.get("created_by"),
    })
}

fn ledger_from_row(
    row: tokio_postgres::Row,
) -> Result<ironclaw_engine::types::budget::BudgetLedger, DatabaseError> {
    let tokens_used: i64 = row.get("tokens_used");
    Ok(ironclaw_engine::types::budget::BudgetLedger {
        budget_id: ironclaw_engine::types::budget::BudgetId(row.get("budget_id")),
        period_start: row.get("period_start"),
        period_end: row.get("period_end"),
        spent_usd: row.get("spent_usd"),
        reserved_usd: row.get("reserved_usd"),
        tokens_used: tokens_used as u64,
        updated_at: row.get("updated_at"),
    })
}

fn rehydrate_scope(
    kind: &str,
    scope_id: &str,
    user_id: String,
) -> Result<ironclaw_engine::types::budget::BudgetScope, DatabaseError> {
    use ironclaw_engine::types::budget::{BackgroundKind, BudgetScope};

    Ok(match kind {
        "user" => BudgetScope::User { user_id },
        "project" => BudgetScope::Project {
            user_id,
            project_id: ironclaw_engine::types::project::ProjectId(
                Uuid::parse_str(scope_id)
                    .map_err(|e| DatabaseError::Query(format!("project scope id: {e}")))?,
            ),
        },
        "mission" => BudgetScope::Mission {
            user_id,
            mission_id: ironclaw_engine::types::mission::MissionId(
                Uuid::parse_str(scope_id)
                    .map_err(|e| DatabaseError::Query(format!("mission scope id: {e}")))?,
            ),
        },
        "thread" => BudgetScope::Thread {
            user_id,
            thread_id: ironclaw_engine::ThreadId(
                Uuid::parse_str(scope_id)
                    .map_err(|e| DatabaseError::Query(format!("thread scope id: {e}")))?,
            ),
        },
        "background" => {
            let (kind_str, corr) = scope_id
                .split_once(':')
                .ok_or_else(|| DatabaseError::Query("background scope malformed".into()))?;
            let bk = match kind_str {
                "heartbeat" => BackgroundKind::Heartbeat,
                "routine_lightweight" => BackgroundKind::RoutineLightweight,
                "routine_standard" => BackgroundKind::RoutineStandard,
                "mission_tick" => BackgroundKind::MissionTick,
                "container_job" => BackgroundKind::ContainerJob,
                "user_initiated" => BackgroundKind::UserInitiated,
                other => {
                    return Err(DatabaseError::Query(format!(
                        "unknown background kind '{other}'"
                    )));
                }
            };
            BudgetScope::BackgroundInvocation {
                user_id,
                kind: bk,
                correlation_id: corr.to_string(),
            }
        }
        other => {
            return Err(DatabaseError::Query(format!(
                "unknown scope_kind '{other}'"
            )));
        }
    })
}

fn rehydrate_period(
    kind: &str,
    tz: Option<String>,
    unit: Option<String>,
) -> Result<ironclaw_engine::types::budget::BudgetPeriod, DatabaseError> {
    use ironclaw_engine::types::budget::{BudgetPeriod, PeriodUnit};

    Ok(match kind {
        "per_invocation" => BudgetPeriod::PerInvocation,
        "rolling_24h" => BudgetPeriod::Rolling24h,
        "calendar" => {
            let tz = tz.ok_or_else(|| DatabaseError::Query("calendar period missing tz".into()))?;
            let unit = match unit
                .as_deref()
                .ok_or_else(|| DatabaseError::Query("calendar period missing unit".into()))?
            {
                "day" => PeriodUnit::Day,
                "week" => PeriodUnit::Week,
                "month" => PeriodUnit::Month,
                other => {
                    return Err(DatabaseError::Query(format!(
                        "unknown period_unit '{other}'"
                    )));
                }
            };
            BudgetPeriod::Calendar { tz, unit }
        }
        other => {
            return Err(DatabaseError::Query(format!(
                "unknown period_kind '{other}'"
            )));
        }
    })
}
