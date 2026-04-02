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
    ApiTokenRecord, ConversationStore, Database, JobStore, RoutineStore, SandboxStore,
    SettingsStore, ToolFailureStore, UserRecord, UserStore, WorkspaceMemberRecord,
    WorkspaceMembership, WorkspaceMgmtStore, WorkspaceRecord, WorkspaceStore,
};
use crate::error::{DatabaseError, WorkspaceError};
use crate::history::{
    AgentJobRecord, AgentJobSummary, ConversationMessage, ConversationSummary, JobEventRecord,
    LlmCallRecord, SandboxJobRecord, SandboxJobSummary, SettingRow, Store,
};
use crate::workspace::{
    MemoryChunk, MemoryDocument, Repository, SearchConfig, SearchResult, WorkspaceEntry,
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
        workspace_id: Option<Uuid>,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation(channel, user_id, workspace_id, thread_id)
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

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        workspace_id: Option<Uuid>,
        thread_id: Option<&str>,
        source_channel: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        self.store
            .ensure_conversation(
                id,
                channel,
                user_id,
                workspace_id,
                thread_id,
                source_channel,
            )
            .await
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        workspace_id: Option<Uuid>,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_conversations_with_preview(user_id, workspace_id, channel, limit)
            .await
    }

    async fn list_conversations_all_channels(
        &self,
        user_id: &str,
        workspace_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_conversations_all_channels(user_id, workspace_id, limit)
            .await
    }

    async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
        workspace_id: Option<Uuid>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_routine_conversation(routine_id, routine_name, user_id, workspace_id)
            .await
    }

    async fn find_routine_conversation(
        &self,
        routine_id: Uuid,
        user_id: &str,
        workspace_id: Option<Uuid>,
    ) -> Result<Option<Uuid>, DatabaseError> {
        self.store
            .find_routine_conversation(routine_id, user_id, workspace_id)
            .await
    }

    async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
        workspace_id: Option<Uuid>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_heartbeat_conversation(user_id, workspace_id)
            .await
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        workspace_id: Option<Uuid>,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_assistant_conversation(user_id, workspace_id, channel)
            .await
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        workspace_id: Option<Uuid>,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation_with_metadata(channel, user_id, workspace_id, metadata)
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
        workspace_id: Option<Uuid>,
    ) -> Result<bool, DatabaseError> {
        self.store
            .conversation_belongs_to_user(conversation_id, user_id, workspace_id)
            .await
    }

    async fn get_conversation_workspace_id(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<Uuid>, DatabaseError> {
        self.store
            .get_conversation_workspace_id(conversation_id)
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

    async fn list_agent_jobs_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        self.store.list_agent_jobs_for_workspace(workspace_id).await
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

    async fn agent_job_summary_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<AgentJobSummary, DatabaseError> {
        self.store
            .agent_job_summary_for_workspace(workspace_id)
            .await
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

    async fn list_sandbox_jobs_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store
            .list_sandbox_jobs_for_workspace(workspace_id)
            .await
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary_for_user(user_id).await
    }

    async fn sandbox_job_summary_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        self.store
            .sandbox_job_summary_for_workspace(workspace_id)
            .await
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
        workspace_id: Option<Uuid>,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.store
            .get_routine_by_name(user_id, workspace_id, name)
            .await
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_routines(user_id).await
    }

    async fn list_routines_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_routines_for_workspace(workspace_id).await
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

    async fn get_setting_for_workspace(
        &self,
        workspace_id: Uuid,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.store
            .get_setting_for_workspace(workspace_id, key)
            .await
    }

    async fn get_setting_full_for_workspace(
        &self,
        workspace_id: Uuid,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        self.store
            .get_setting_full_for_workspace(workspace_id, key)
            .await
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store.set_setting(user_id, key, value).await
    }

    async fn set_setting_for_workspace(
        &self,
        workspace_id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .set_setting_for_workspace(workspace_id, key, value)
            .await
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        self.store.delete_setting(user_id, key).await
    }

    async fn delete_setting_for_workspace(
        &self,
        workspace_id: Uuid,
        key: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .delete_setting_for_workspace(workspace_id, key)
            .await
    }

    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        self.store.list_settings(user_id).await
    }

    async fn list_settings_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<SettingRow>, DatabaseError> {
        self.store.list_settings_for_workspace(workspace_id).await
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.store.get_all_settings(user_id).await
    }

    async fn get_all_settings_for_workspace(
        &self,
        workspace_id: Uuid,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.store
            .get_all_settings_for_workspace(workspace_id)
            .await
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.store.set_all_settings(user_id, settings).await
    }

    async fn set_all_settings_for_workspace(
        &self,
        workspace_id: Uuid,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.store
            .set_all_settings_for_workspace(workspace_id, settings)
            .await
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        self.store.has_settings(user_id).await
    }

    async fn has_settings_for_workspace(&self, workspace_id: Uuid) -> Result<bool, DatabaseError> {
        self.store.has_settings_for_workspace(workspace_id).await
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

fn row_to_workspace(row: &tokio_postgres::Row) -> WorkspaceRecord {
    WorkspaceRecord {
        id: row.get("id"),
        name: row.get("name"),
        slug: row.get("slug"),
        description: row.get("description"),
        status: row.get("status"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        created_by: row.get("created_by"),
        settings: row.get("settings"),
    }
}

#[async_trait]
impl WorkspaceMgmtStore for PgBackend {
    async fn create_workspace(
        &self,
        name: &str,
        slug: &str,
        description: &str,
        created_by: &str,
        settings: &serde_json::Value,
    ) -> Result<WorkspaceRecord, DatabaseError> {
        let mut client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let tx = client.transaction().await?;
        let row = tx
            .query_one(
                r#"
                INSERT INTO workspaces (id, name, slug, description, created_by, settings)
                VALUES ($1, $2, $3, $4, $5, $6)
                RETURNING id, name, slug, description, status, created_at, updated_at, created_by, settings
                "#,
                &[&Uuid::new_v4(), &name, &slug, &description, &created_by, &settings],
            )
            .await?;
        let workspace = row_to_workspace(&row);
        tx.execute(
            r#"
            INSERT INTO workspace_members (workspace_id, user_id, role, invited_by)
            VALUES ($1, $2, 'owner', $2)
            "#,
            &[&workspace.id, &created_by],
        )
        .await?;
        tx.commit().await?;
        Ok(workspace)
    }

    async fn get_workspace(&self, id: Uuid) -> Result<Option<WorkspaceRecord>, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let row = client
            .query_opt(
                r#"
                SELECT id, name, slug, description, status, created_at, updated_at, created_by, settings
                FROM workspaces
                WHERE id = $1
                "#,
                &[&id],
            )
            .await?;
        Ok(row.map(|row| row_to_workspace(&row)))
    }

    async fn get_workspace_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<WorkspaceRecord>, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let row = client
            .query_opt(
                r#"
                SELECT id, name, slug, description, status, created_at, updated_at, created_by, settings
                FROM workspaces
                WHERE slug = $1
                "#,
                &[&slug],
            )
            .await?;
        Ok(row.map(|row| row_to_workspace(&row)))
    }

    async fn list_workspaces_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<WorkspaceMembership>, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let rows = client
            .query(
                r#"
                SELECT w.id, w.name, w.slug, w.description, w.status, w.created_at, w.updated_at, w.created_by, w.settings,
                       wm.role
                FROM workspace_members wm
                JOIN workspaces w ON w.id = wm.workspace_id
                WHERE wm.user_id = $1
                  AND w.status != 'archived'
                ORDER BY w.created_at DESC
                "#,
                &[&user_id],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| WorkspaceMembership {
                workspace: row_to_workspace(&row),
                role: row.get("role"),
            })
            .collect())
    }

    async fn update_workspace(
        &self,
        id: Uuid,
        name: &str,
        description: &str,
        settings: &serde_json::Value,
    ) -> Result<Option<WorkspaceRecord>, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let row = client
            .query_opt(
                r#"
                UPDATE workspaces
                SET name = $2, description = $3, settings = $4, updated_at = now()
                WHERE id = $1
                RETURNING id, name, slug, description, status, created_at, updated_at, created_by, settings
                "#,
                &[&id, &name, &description, &settings],
            )
            .await?;
        Ok(row.map(|row| row_to_workspace(&row)))
    }

    async fn archive_workspace(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let updated = client
            .execute(
                "UPDATE workspaces SET status = 'archived', updated_at = now() WHERE id = $1",
                &[&id],
            )
            .await?;
        Ok(updated > 0)
    }

    async fn add_workspace_member(
        &self,
        workspace_id: Uuid,
        user_id: &str,
        role: &str,
        invited_by: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        client
            .execute(
                r#"
                INSERT INTO workspace_members (workspace_id, user_id, role, invited_by)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (workspace_id, user_id) DO UPDATE SET
                    role = EXCLUDED.role,
                    invited_by = EXCLUDED.invited_by,
                    joined_at = now()
                "#,
                &[&workspace_id, &user_id, &role, &invited_by],
            )
            .await?;
        Ok(())
    }

    async fn remove_workspace_member(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let deleted = client
            .execute(
                "DELETE FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
                &[&workspace_id, &user_id],
            )
            .await?;
        Ok(deleted > 0)
    }

    async fn list_workspace_members(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<(UserRecord, WorkspaceMemberRecord)>, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let rows = client
            .query(
                r#"
                SELECT
                    u.id, u.email, u.display_name, u.status, u.role AS user_role, u.created_at, u.updated_at,
                    u.last_login_at, u.created_by, u.metadata,
                    wm.workspace_id, wm.user_id AS member_user_id, wm.role AS member_role, wm.joined_at, wm.invited_by
                FROM workspace_members wm
                JOIN users u ON u.id = wm.user_id
                WHERE wm.workspace_id = $1
                ORDER BY wm.joined_at ASC
                "#,
                &[&workspace_id],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    UserRecord {
                        id: row.get("id"),
                        email: row.get("email"),
                        display_name: row.get("display_name"),
                        status: row.get("status"),
                        role: row.get("user_role"),
                        created_at: row.get("created_at"),
                        updated_at: row.get("updated_at"),
                        last_login_at: row.get("last_login_at"),
                        created_by: row.get("created_by"),
                        metadata: row.get("metadata"),
                    },
                    WorkspaceMemberRecord {
                        workspace_id: row.get("workspace_id"),
                        user_id: row.get("member_user_id"),
                        role: row.get("member_role"),
                        joined_at: row.get("joined_at"),
                        invited_by: row.get("invited_by"),
                    },
                )
            })
            .collect())
    }

    async fn get_member_role(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let row = client
            .query_opt(
                "SELECT role FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
                &[&workspace_id, &user_id],
            )
            .await?;
        Ok(row.map(|row| row.get("role")))
    }

    async fn is_last_workspace_owner(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let row = client
            .query_one(
                r#"
                SELECT
                    COALESCE(SUM(CASE WHEN user_id = $2 AND role = 'owner' THEN 1 ELSE 0 END), 0) AS target_is_owner,
                    COALESCE(SUM(CASE WHEN role = 'owner' THEN 1 ELSE 0 END), 0) AS owner_count
                FROM workspace_members
                WHERE workspace_id = $1
                "#,
                &[&workspace_id, &user_id],
            )
            .await?;
        let target_is_owner: i64 = row.get("target_is_owner");
        let owner_count: i64 = row.get("owner_count");
        Ok(target_is_owner > 0 && owner_count <= 1)
    }

    async fn update_member_role(
        &self,
        workspace_id: Uuid,
        user_id: &str,
        role: &str,
    ) -> Result<bool, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let updated = client
            .execute(
                "UPDATE workspace_members SET role = $3 WHERE workspace_id = $1 AND user_id = $2",
                &[&workspace_id, &user_id, &role],
            )
            .await?;
        Ok(updated > 0)
    }

    async fn is_workspace_member(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let client = self
            .pool()
            .get()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to get client: {e}")))?;
        let row = client
            .query_opt(
                "SELECT 1 FROM workspace_members WHERE workspace_id = $1 AND user_id = $2",
                &[&workspace_id, &user_id],
            )
            .await?;
        Ok(row.is_some())
    }
}
