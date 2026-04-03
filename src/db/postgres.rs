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
    ApiTokenRecord, ConversationStore, Database, IdentityStore, JobStore, RoutineStore,
    SandboxStore, SettingsStore, ToolFailureStore, UserIdentityRecord, UserRecord, UserStore,
    WorkspaceStore, structured,
    structured::{Aggregation, CollectionSchema, Filter, Record, StructuredStore},
};
use crate::error::{DatabaseError, WorkspaceError};
use crate::history::{
    AgentJobRecord, AgentJobSummary, ConversationMessage, ConversationSummary, JobEventRecord,
    LlmCallRecord, SandboxJobRecord, SandboxJobSummary, SettingRow, Store,
};
use crate::workspace::{
    DocumentVersion, MemoryChunk, MemoryDocument, Repository, SearchConfig, SearchResult,
    VersionSummary, WorkspaceEntry,
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

        tx.commit().await?;
        Ok(())
    }
}

// ==================== StructuredStore ====================

/// Convert a tokio_postgres Row into a structured Record.
fn pg_row_to_record(row: &tokio_postgres::Row) -> Result<Record, DatabaseError> {
    let id: Uuid = row.get("id");
    let user_id: String = row.get("user_id");
    let collection: String = row.get("collection");
    let data: serde_json::Value = row.get("data");
    let created_at: DateTime<Utc> = row.get("created_at");
    let updated_at: DateTime<Utc> = row.get("updated_at");

    Ok(Record {
        id,
        user_id,
        collection,
        data,
        created_at,
        updated_at,
    })
}

/// Boxed dynamic SQL parameter for tokio_postgres queries.
type PgParam = Box<dyn tokio_postgres::types::ToSql + Sync + Send>;

/// Resolve a filter field name to its SQL expression.
///
/// Special fields:
/// - `created_at` / `updated_at` -> use the DB column cast to text for comparison
/// - Dot-notation (e.g. `_lineage.source`) -> nested JSONB access (`data->'_lineage'->>'source'`)
/// - Everything else -> `data->>'field'`
///
/// Returns `(sql_expr, is_timestamp_column)`. When `is_timestamp_column` is true,
/// callers should cast the filter value to timestamp for proper comparison.
fn resolve_filter_field(field: &str) -> Result<(String, bool), DatabaseError> {
    // DB column fields.
    if field == "created_at" || field == "updated_at" {
        return Ok((format!("{field}::text"), true));
    }

    // Dot-notation for nested JSONB: `parent.child` -> `data->'parent'->>'child'`.
    if let Some((parent, child)) = field.split_once('.') {
        // Validate both segments to prevent SQL injection.
        validate_filter_field_segment(parent)?;
        validate_filter_field_segment(child)?;
        return Ok((format!("data->'{}'->>'{}'", parent, child), false));
    }

    // Regular data field.
    structured::validate_field_name(field).map_err(|e| DatabaseError::Query(e.to_string()))?;
    Ok((format!("data->>'{field}'"), false))
}

/// Validate a single segment of a filter field name (for dot-notation).
///
/// Allows system-field prefixes (starting with `_`) in addition to regular identifiers.
fn validate_filter_field_segment(name: &str) -> Result<(), DatabaseError> {
    if name.is_empty() || name.len() > 64 {
        return Err(DatabaseError::Query(format!(
            "filter field segment '{name}' must be 1-64 characters"
        )));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(DatabaseError::Query(format!(
            "filter field segment '{name}' contains invalid characters"
        )));
    }
    Ok(())
}

/// Build filter WHERE clauses and collect parameters for a set of filters.
///
/// Returns (where_clauses, params) where where_clauses is a Vec of SQL fragments
/// and params is the collected parameter values.
///
/// Field names are validated to prevent SQL injection since they are interpolated
/// directly into query strings (PostgreSQL doesn't support parameterized column names).
fn build_filters(
    filters: &[Filter],
    start_idx: i32,
) -> Result<(Vec<String>, Vec<PgParam>), DatabaseError> {
    let mut clauses = Vec::new();
    let mut params: Vec<PgParam> = Vec::new();
    let mut idx = start_idx;

    for filter in filters {
        let (sql_field, _is_ts) = resolve_filter_field(&filter.field)?;

        let make_compare = |op: &str, idx: &mut i32| -> (String, Vec<PgParam>) {
            let val = json_value_to_text_string(&filter.value);
            let clause = format!("{sql_field} {op} ${}", *idx);
            *idx += 1;
            (clause, vec![Box::new(val) as PgParam])
        };

        match filter.op {
            structured::FilterOp::IsNull => {
                clauses.push(format!("{sql_field} IS NULL"));
            }
            structured::FilterOp::IsNotNull => {
                clauses.push(format!("{sql_field} IS NOT NULL"));
            }
            structured::FilterOp::Eq => {
                let (clause, p) = make_compare("=", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Neq => {
                let (clause, p) = make_compare("!=", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Gt => {
                let (clause, p) = make_compare(">", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Gte => {
                let (clause, p) = make_compare(">=", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Lt => {
                let (clause, p) = make_compare("<", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Lte => {
                let (clause, p) = make_compare("<=", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Between => {
                let arr = filter.value.as_array().ok_or_else(|| {
                    DatabaseError::Query("Between filter requires an array of [lo, hi]".to_string())
                })?;
                if arr.len() != 2 {
                    return Err(DatabaseError::Query(
                        "Between filter requires exactly 2 elements".to_string(),
                    ));
                }
                clauses.push(format!("{sql_field} BETWEEN ${idx} AND ${}", idx + 1));
                params.push(Box::new(json_value_to_text_string(&arr[0])));
                params.push(Box::new(json_value_to_text_string(&arr[1])));
                idx += 2;
            }
            structured::FilterOp::In => {
                let arr = filter.value.as_array().ok_or_else(|| {
                    DatabaseError::Query("In filter requires an array value".to_string())
                })?;
                if arr.is_empty() {
                    clauses.push("FALSE".to_string());
                } else {
                    let placeholders: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, _)| format!("${}", idx + i as i32))
                        .collect();
                    clauses.push(format!("{sql_field} IN ({})", placeholders.join(", ")));
                    for item in arr {
                        params.push(Box::new(json_value_to_text_string(item)));
                    }
                    idx += arr.len() as i32;
                }
            }
        }
    }

    Ok((clauses, params))
}

/// Convert a JSON value to its text representation as it would appear from
/// PostgreSQL's JSONB `data->>'field'` operator (which always returns text).
fn json_value_to_text_string(value: &serde_json::Value) -> String {
    structured::json_to_text(value)
}

#[async_trait]
impl StructuredStore for PgBackend {
    async fn register_collection(
        &self,
        user_id: &str,
        schema: &CollectionSchema,
    ) -> Result<(), DatabaseError> {
        CollectionSchema::validate_name(&schema.collection)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        schema
            .validate_source_scope()
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let schema_json = serde_json::to_value(schema)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let conn = self.store.conn().await?;
        conn.execute(
            r#"
            INSERT INTO structured_schemas (user_id, collection, schema, description)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id, collection) DO UPDATE SET
                schema = EXCLUDED.schema,
                description = EXCLUDED.description
            "#,
            &[
                &user_id,
                &schema.collection.as_str(),
                &schema_json,
                &schema.description,
            ],
        )
        .await?;

        Ok(())
    }

    async fn get_collection_schema(
        &self,
        user_id: &str,
        collection: &str,
    ) -> Result<CollectionSchema, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT schema FROM structured_schemas WHERE user_id = $1 AND collection = $2",
                &[&user_id, &collection],
            )
            .await?;

        let row = rows.first().ok_or_else(|| DatabaseError::NotFound {
            entity: "collection".to_string(),
            id: collection.to_string(),
        })?;

        let schema_json: serde_json::Value = row.get("schema");
        serde_json::from_value(schema_json).map_err(|e| DatabaseError::Serialization(e.to_string()))
    }

    async fn list_collections(
        &self,
        user_id: &str,
    ) -> Result<Vec<CollectionSchema>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT schema FROM structured_schemas WHERE user_id = $1 ORDER BY collection",
                &[&user_id],
            )
            .await?;

        let mut schemas = Vec::with_capacity(rows.len());
        for row in &rows {
            let schema_json: serde_json::Value = row.get("schema");
            let schema: CollectionSchema = serde_json::from_value(schema_json)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            schemas.push(schema);
        }
        Ok(schemas)
    }

    async fn drop_collection(&self, user_id: &str, collection: &str) -> Result<(), DatabaseError> {
        let conn = self.store.conn().await?;
        let n = conn
            .execute(
                "DELETE FROM structured_schemas WHERE user_id = $1 AND collection = $2",
                &[&user_id, &collection],
            )
            .await?;

        if n == 0 {
            return Err(DatabaseError::NotFound {
                entity: "collection".to_string(),
                id: collection.to_string(),
            });
        }
        Ok(())
    }

    async fn insert_record(
        &self,
        user_id: &str,
        collection: &str,
        data: serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let schema = self.get_collection_schema(user_id, collection).await?;
        let validated = schema
            .validate_record(&data)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let id = Uuid::new_v4();
        let conn = self.store.conn().await?;
        conn.execute(
            r#"
            INSERT INTO structured_records (id, user_id, collection, data)
            VALUES ($1, $2, $3, $4)
            "#,
            &[&id, &user_id, &collection, &validated],
        )
        .await?;

        Ok(id)
    }

    async fn get_record(&self, user_id: &str, record_id: Uuid) -> Result<Record, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, user_id, collection, data, created_at, updated_at
                FROM structured_records
                WHERE id = $1 AND user_id = $2
                "#,
                &[&record_id, &user_id],
            )
            .await?;

        let row = rows.first().ok_or_else(|| DatabaseError::NotFound {
            entity: "record".to_string(),
            id: record_id.to_string(),
        })?;

        pg_row_to_record(row)
    }

    async fn update_record(
        &self,
        user_id: &str,
        record_id: Uuid,
        updates: serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let mut conn = self.store.conn().await?;
        let tx = conn.transaction().await?;

        // Lock the record row to prevent concurrent modification.
        let rows = tx
            .query(
                r#"
                SELECT id, user_id, collection, data, created_at, updated_at
                FROM structured_records
                WHERE id = $1 AND user_id = $2
                FOR UPDATE
                "#,
                &[&record_id, &user_id],
            )
            .await?;

        let row = rows.first().ok_or_else(|| DatabaseError::NotFound {
            entity: "record".to_string(),
            id: record_id.to_string(),
        })?;
        let existing = pg_row_to_record(row)?;

        // Fetch schema (read-only, doesn't need locking).
        let schema_rows = tx
            .query(
                "SELECT schema FROM structured_schemas WHERE user_id = $1 AND collection = $2",
                &[&user_id, &existing.collection],
            )
            .await?;

        let schema_row = schema_rows.first().ok_or_else(|| DatabaseError::NotFound {
            entity: "collection".to_string(),
            id: existing.collection.clone(),
        })?;
        let schema: structured::CollectionSchema =
            serde_json::from_value(schema_row.get::<_, serde_json::Value>("schema"))
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        // Validate the partial update.
        let validated_updates = schema
            .validate_partial(&updates)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // Merge updates into existing data.
        let mut merged = existing.data.clone();
        if let (Some(base), Some(patch)) = (merged.as_object_mut(), validated_updates.as_object()) {
            for (k, v) in patch {
                base.insert(k.clone(), v.clone());
            }
        }

        let n = tx
            .execute(
                r#"
                UPDATE structured_records
                SET data = $1, updated_at = NOW()
                WHERE id = $2 AND user_id = $3
                "#,
                &[&merged, &record_id, &user_id],
            )
            .await?;

        if n == 0 {
            return Err(DatabaseError::NotFound {
                entity: "record".to_string(),
                id: record_id.to_string(),
            });
        }

        tx.commit().await?;
        Ok(())
    }

    async fn delete_record(&self, user_id: &str, record_id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.store.conn().await?;
        let n = conn
            .execute(
                "DELETE FROM structured_records WHERE id = $1 AND user_id = $2",
                &[&record_id, &user_id],
            )
            .await?;

        if n == 0 {
            return Err(DatabaseError::NotFound {
                entity: "record".to_string(),
                id: record_id.to_string(),
            });
        }
        Ok(())
    }

    async fn query_records(
        &self,
        user_id: &str,
        collection: &str,
        filters: &[Filter],
        order_by: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Record>, DatabaseError> {
        // Cap at 1000 for normal queries, but allow uncapped for internal
        // callers that pass usize::MAX (e.g., aggregation fallbacks).
        let capped_limit = if limit == usize::MAX {
            i64::MAX
        } else {
            limit.min(1000) as i64
        };

        // Start building the query. Params $1 = user_id, $2 = collection.
        let mut sql = String::from(
            "SELECT id, user_id, collection, data, created_at, updated_at \
             FROM structured_records WHERE user_id = $1 AND collection = $2",
        );
        let mut params: Vec<PgParam> = Vec::new();
        params.push(Box::new(user_id.to_string()));
        params.push(Box::new(collection.to_string()));

        // Build filter clauses starting at $3.
        let (filter_clauses, filter_params) = build_filters(filters, 3)?;
        for clause in &filter_clauses {
            sql.push_str(" AND ");
            sql.push_str(clause);
        }
        params.extend(filter_params);

        // ORDER BY
        match order_by {
            Some(field) => {
                structured::validate_field_name(field)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                // Top-level columns are used directly, not extracted from JSON.
                if field == "created_at" || field == "updated_at" {
                    sql.push_str(&format!(" ORDER BY {field}"));
                } else {
                    sql.push_str(&format!(" ORDER BY data->>'{field}'"));
                }
            }
            None => {
                sql.push_str(" ORDER BY created_at DESC");
            }
        }

        // LIMIT
        let limit_idx = params.len() as i32 + 1;
        sql.push_str(&format!(" LIMIT ${limit_idx}"));
        params.push(Box::new(capped_limit));

        // Build reference slice for tokio_postgres.
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let conn = self.store.conn().await?;
        let rows = conn.query(&sql, &param_refs).await?;

        let mut records = Vec::with_capacity(rows.len());
        for row in &rows {
            records.push(pg_row_to_record(row)?);
        }
        Ok(records)
    }

    async fn aggregate(
        &self,
        user_id: &str,
        collection: &str,
        aggregation: &Aggregation,
    ) -> Result<serde_json::Value, DatabaseError> {
        let group_by = &aggregation.group_by;

        // Validate field names to prevent SQL injection (they are interpolated
        // directly into query strings).
        if let Some(field) = &aggregation.field {
            structured::validate_field_name(field)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }
        if let Some(group_field) = group_by {
            structured::validate_field_name(group_field)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        // Build the aggregation expression.
        let agg_expr = match aggregation.operation {
            structured::AggOp::Count => "COUNT(*)".to_string(),
            structured::AggOp::Sum => {
                let field = aggregation
                    .field
                    .as_deref()
                    .ok_or_else(|| DatabaseError::Query("Sum requires a field".to_string()))?;
                format!("SUM((data->>'{field}')::numeric)")
            }
            structured::AggOp::Avg => {
                let field = aggregation
                    .field
                    .as_deref()
                    .ok_or_else(|| DatabaseError::Query("Avg requires a field".to_string()))?;
                format!("AVG((data->>'{field}')::numeric)")
            }
            structured::AggOp::Min => {
                let field = aggregation
                    .field
                    .as_deref()
                    .ok_or_else(|| DatabaseError::Query("Min requires a field".to_string()))?;
                format!("MIN(data->>'{field}')")
            }
            structured::AggOp::Max => {
                let field = aggregation
                    .field
                    .as_deref()
                    .ok_or_else(|| DatabaseError::Query("Max requires a field".to_string()))?;
                format!("MAX(data->>'{field}')")
            }
        };

        // Start building query. $1 = user_id, $2 = collection.
        let mut sql = if let Some(group_field) = group_by {
            format!(
                "SELECT data->>'{group_field}' AS group_key, {agg_expr} AS result \
                 FROM structured_records WHERE user_id = $1 AND collection = $2"
            )
        } else {
            format!(
                "SELECT {agg_expr} AS result \
                 FROM structured_records WHERE user_id = $1 AND collection = $2"
            )
        };

        let mut params: Vec<PgParam> = Vec::new();
        params.push(Box::new(user_id.to_string()));
        params.push(Box::new(collection.to_string()));

        // Apply filters.
        let (filter_clauses, filter_params) = build_filters(&aggregation.filters, 3)?;
        for clause in &filter_clauses {
            sql.push_str(" AND ");
            sql.push_str(clause);
        }
        params.extend(filter_params);

        // GROUP BY
        if let Some(group_field) = group_by {
            sql.push_str(&format!(" GROUP BY data->>'{group_field}'"));
        }

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
            .iter()
            .map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync))
            .collect();

        let conn = self.store.conn().await?;
        let rows = conn.query(&sql, &param_refs).await?;

        if group_by.is_some() {
            // Grouped result: return an object { "group_key": result, ... }
            let mut result_map = serde_json::Map::new();
            for row in &rows {
                let key: Option<String> = row.get("group_key");
                let key_str = key.unwrap_or_else(|| "null".to_string());

                // The result type depends on the aggregation operation.
                let value = extract_agg_value(row, &aggregation.operation)?;
                result_map.insert(key_str, value);
            }
            Ok(serde_json::Value::Object(result_map))
        } else {
            // Single result.
            let row = rows
                .first()
                .ok_or_else(|| DatabaseError::Query("Aggregation returned no rows".to_string()))?;
            extract_agg_value(row, &aggregation.operation)
        }
    }
}

/// Extract the aggregation result value from a PostgreSQL row.
fn extract_agg_value(
    row: &tokio_postgres::Row,
    op: &structured::AggOp,
) -> Result<serde_json::Value, DatabaseError> {
    match op {
        structured::AggOp::Count => {
            let count: i64 = row.get("result");
            Ok(serde_json::json!(count))
        }
        structured::AggOp::Sum | structured::AggOp::Avg => {
            // SUM/AVG of numeric returns Option<Decimal>
            let val: Option<Decimal> = row.get("result");
            match val {
                Some(d) => {
                    use rust_decimal::prelude::ToPrimitive;
                    let f = d.to_f64().ok_or_else(|| {
                        DatabaseError::Query(format!("Cannot convert aggregate result {d} to f64"))
                    })?;
                    Ok(serde_json::json!(f))
                }
                None => Ok(serde_json::Value::Null),
            }
        }
        structured::AggOp::Min | structured::AggOp::Max => {
            let val: Option<String> = row.get("result");
            match val {
                Some(s) => {
                    if let Ok(n) = s.parse::<f64>() {
                        Ok(serde_json::json!(n))
                    } else {
                        Ok(serde_json::json!(s))
                    }
                }
                None => Ok(serde_json::Value::Null),
            }
        }
    }
}
