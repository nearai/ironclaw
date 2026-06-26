//! Guardrail test: `load_identity_candidates` must read identity files
//! CONCURRENTLY, never in a serial cross-region loop.
//!
//! Strategy: wrap a `Database` in a `ConcurrencyProbeDb` that sleeps 40 ms
//! inside `get_document_by_path` while holding an in-flight counter. Serial
//! code (one `.await?` per loop iteration) keeps `max_observed == 1`; parallel
//! code (`futures::future::try_join_all`) shows `max_observed >= 2`. The test
//! asserts `max_observed >= 2` — it FAILED on the original serial loop (proving
//! the latency bug) and PASSES after parallelization; it also fails forever if
//! a serial regression returns.
//!
//! Backend-agnostic by construction: the probe instruments the `Database`
//! trait boundary (`get_document_by_path`) that EVERY backend flows through, so
//! the assertion measures how many concurrent store reads the production code
//! issues — identical for postgres, libsql, and in-memory. The inner LibSQL
//! backend is only a convenient real data store; it sits BELOW the probe's
//! counter and does not affect the measurement. The production beneficiary is
//! POSTGRES (hosted, cross-region ~100-200ms/RTT): N serial round-trips collapse
//! to ~1. In-memory saves ~0 wall-clock but stays correct. `libsql` is the
//! feature simply because it provides an embeddable store for the test rig.
//!
//! No production files are touched.

#![cfg(feature = "libsql")]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use ironclaw::agent::BrokenTool;
use ironclaw::agent::routine::RunStatus;
use ironclaw::agent::routine::{Routine, RoutineRun};
use ironclaw::context::{ActionRecord, JobContext, JobState};
use ironclaw::db::{
    AdminUsageSummary, ApiTokenRecord, ConversationStore, Database, IdentityStore, JobStore,
    PairingApprovalRecord, PairingRequestRecord, RoutineStore, SandboxStore, SettingsStore,
    ToolFailureStore, UserIdentityRecord, UserRecord, UserStore, UserSummaryStats, UserUsageStats,
    WorkspaceStore,
};
use ironclaw::error::{DatabaseError, WorkspaceError};
use ironclaw::history::{
    AgentJobRecord, AgentJobSummary, ConversationMessage, ConversationSummary, JobEventRecord,
    LlmCallRecord, SandboxJobRecord, SandboxJobSummary, SettingRow,
};
use ironclaw::ownership::UserId;
use ironclaw::workspace::{
    ChunkWrite, DocumentVersion, MemoryChunk, MemoryDocument, SearchConfig, SearchResult,
    VersionSummary, Workspace, WorkspaceEntry, WorkspaceIdentityContextSource, paths,
};
use ironclaw_loop_support::HostIdentityContextSource;
use ironclaw_turns::run_profile::{LoopRunContext, PersonalContextPolicy, PromptMode};

// ---------------------------------------------------------------------------
// ConcurrencyProbeDb
// ---------------------------------------------------------------------------

/// Wraps any `Arc<dyn Database>`, adding a 40 ms sleep + in-flight counter
/// to `get_document_by_path`. All other methods forward directly.
struct ConcurrencyProbeDb {
    inner: Arc<dyn Database>,
    current: Arc<AtomicUsize>,
    max: Arc<AtomicUsize>,
}

impl ConcurrencyProbeDb {
    fn new(inner: Arc<dyn Database>, current: Arc<AtomicUsize>, max: Arc<AtomicUsize>) -> Self {
        Self {
            inner,
            current,
            max,
        }
    }
}

// ---------------------------------------------------------------------------
// WorkspaceStore — probe lives here
// ---------------------------------------------------------------------------

#[async_trait]
impl WorkspaceStore for ConcurrencyProbeDb {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        // Count this call as in-flight
        let prev = self.current.fetch_add(1, Ordering::SeqCst);
        let cur = prev + 1;
        // Update the high-water mark
        self.max.fetch_max(cur, Ordering::SeqCst);
        // Sleep long enough that concurrent calls overlap
        tokio::time::sleep(Duration::from_millis(40)).await;
        // No longer in-flight
        self.current.fetch_sub(1, Ordering::SeqCst);
        self.inner
            .get_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        self.inner.get_document_by_id(id).await
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        self.inner
            .get_or_create_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        self.inner.update_document(id, content).await
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        self.inner
            .delete_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        self.inner
            .list_directory(user_id, agent_id, directory)
            .await
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        self.inner.list_all_paths(user_id, agent_id).await
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        self.inner.list_documents(user_id, agent_id).await
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        self.inner.delete_chunks(document_id).await
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        self.inner
            .insert_chunk(document_id, chunk_index, content, embedding)
            .await
    }

    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[ChunkWrite],
    ) -> Result<(), WorkspaceError> {
        self.inner.replace_chunks(document_id, chunks).await
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        self.inner.update_chunk_embedding(chunk_id, embedding).await
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        self.inner
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
        self.inner
            .hybrid_search(user_id, agent_id, query, embedding, config)
            .await
    }

    async fn update_document_metadata(
        &self,
        id: Uuid,
        metadata: &serde_json::Value,
    ) -> Result<(), WorkspaceError> {
        self.inner.update_document_metadata(id, metadata).await
    }

    async fn find_config_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        self.inner.find_config_documents(user_id, agent_id).await
    }

    async fn save_version(
        &self,
        document_id: Uuid,
        content: &str,
        content_hash: &str,
        changed_by: Option<&str>,
    ) -> Result<i32, WorkspaceError> {
        self.inner
            .save_version(document_id, content, content_hash, changed_by)
            .await
    }

    async fn get_version(
        &self,
        document_id: Uuid,
        version: i32,
    ) -> Result<DocumentVersion, WorkspaceError> {
        self.inner.get_version(document_id, version).await
    }

    async fn list_versions(
        &self,
        document_id: Uuid,
        limit: i64,
    ) -> Result<Vec<VersionSummary>, WorkspaceError> {
        self.inner.list_versions(document_id, limit).await
    }

    async fn get_latest_version_number(
        &self,
        document_id: Uuid,
    ) -> Result<Option<i32>, WorkspaceError> {
        self.inner.get_latest_version_number(document_id).await
    }

    async fn prune_versions(
        &self,
        document_id: Uuid,
        keep_count: i32,
    ) -> Result<u64, WorkspaceError> {
        self.inner.prune_versions(document_id, keep_count).await
    }
}

// ---------------------------------------------------------------------------
// ConversationStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl ConversationStore for ConcurrencyProbeDb {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .create_conversation(channel, user_id, thread_id)
            .await
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.inner.touch_conversation(id).await
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .add_conversation_message(conversation_id, role, content)
            .await
    }

    async fn add_conversation_message_if_empty(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<bool, DatabaseError> {
        self.inner
            .add_conversation_message_if_empty(conversation_id, role, content)
            .await
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
        source_channel: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        self.inner
            .ensure_conversation(id, channel, user_id, thread_id, source_channel)
            .await
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.inner
            .list_conversations_with_preview(user_id, channel, limit)
            .await
    }

    async fn list_conversations_all_channels(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.inner
            .list_conversations_all_channels(user_id, limit)
            .await
    }

    async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .get_or_create_routine_conversation(routine_id, routine_name, user_id)
            .await
    }

    async fn find_routine_conversation(
        &self,
        routine_id: Uuid,
        user_id: &str,
    ) -> Result<Option<Uuid>, DatabaseError> {
        self.inner
            .find_routine_conversation(routine_id, user_id)
            .await
    }

    async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .get_or_create_heartbeat_conversation(user_id)
            .await
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .get_or_create_assistant_conversation(user_id, channel)
            .await
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .create_conversation_with_metadata(channel, user_id, metadata)
            .await
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        self.inner
            .list_conversation_messages_paginated(conversation_id, before, limit)
            .await
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner
            .update_conversation_metadata_field(id, key, value)
            .await
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.inner.get_conversation_metadata(id).await
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        self.inner.list_conversation_messages(conversation_id).await
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.inner
            .conversation_belongs_to_user(conversation_id, user_id)
            .await
    }

    async fn get_conversation_source_channel(
        &self,
        conversation_id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        self.inner
            .get_conversation_source_channel(conversation_id)
            .await
    }
}

// ---------------------------------------------------------------------------
// JobStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl JobStore for ConcurrencyProbeDb {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        self.inner.save_job(ctx).await
    }

    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        self.inner.get_job(id).await
    }

    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.inner
            .update_job_status(id, status, failure_reason)
            .await
    }

    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.inner.mark_job_stuck(id).await
    }

    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        self.inner.get_stuck_jobs().await
    }

    async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        self.inner.list_agent_jobs().await
    }

    async fn list_agent_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        self.inner.list_agent_jobs_for_user(user_id).await
    }

    async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError> {
        self.inner.agent_job_summary().await
    }

    async fn agent_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<AgentJobSummary, DatabaseError> {
        self.inner.agent_job_summary_for_user(user_id).await
    }

    async fn get_agent_job_failure_reason(
        &self,
        id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        self.inner.get_agent_job_failure_reason(id).await
    }

    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError> {
        self.inner.save_action(job_id, action).await
    }

    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        self.inner.get_job_actions(job_id).await
    }

    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        self.inner.record_llm_call(record).await
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
        self.inner
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
        self.inner
            .update_estimation_actuals(id, actual_cost, actual_time_secs, actual_value)
            .await
    }

    async fn create_system_job(&self, user_id: &str, source: &str) -> Result<Uuid, DatabaseError> {
        self.inner.create_system_job(user_id, source).await
    }
}

// ---------------------------------------------------------------------------
// SandboxStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl SandboxStore for ConcurrencyProbeDb {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        self.inner.save_sandbox_job(job).await
    }

    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        self.inner.get_sandbox_job(id).await
    }

    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.inner.list_sandbox_jobs().await
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
        self.inner
            .update_sandbox_job_status(id, status, success, message, started_at, completed_at)
            .await
    }

    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        self.inner.cleanup_stale_sandbox_jobs().await
    }

    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        self.inner.sandbox_job_summary().await
    }

    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.inner.list_sandbox_jobs_for_user(user_id).await
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        self.inner.sandbox_job_summary_for_user(user_id).await
    }

    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.inner
            .sandbox_job_belongs_to_user(job_id, user_id)
            .await
    }

    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        self.inner.update_sandbox_job_mode(id, mode).await
    }

    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        self.inner.get_sandbox_job_mode(id).await
    }

    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner.save_job_event(job_id, event_type, data).await
    }

    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        self.inner.list_job_events(job_id, limit).await
    }
}

// ---------------------------------------------------------------------------
// RoutineStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl RoutineStore for ConcurrencyProbeDb {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.inner.create_routine(routine).await
    }

    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        self.inner.get_routine(id).await
    }

    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.inner.get_routine_by_name(user_id, name).await
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_routines(user_id).await
    }

    async fn list_all_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_all_routines().await
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_event_routines().await
    }

    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_due_cron_routines().await
    }

    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.inner.update_routine(routine).await
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
        self.inner
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
        self.inner.delete_routine(id).await
    }

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        self.inner.create_routine_run(run).await
    }

    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        self.inner
            .complete_routine_run(id, status, result_summary, tokens_used)
            .await
    }

    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        self.inner.list_routine_runs(routine_id, limit).await
    }

    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        self.inner.count_running_routine_runs(routine_id).await
    }

    async fn count_running_routine_runs_batch(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, i64>, DatabaseError> {
        self.inner
            .count_running_routine_runs_batch(routine_ids)
            .await
    }

    async fn batch_get_last_run_status(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, RunStatus>, DatabaseError> {
        self.inner.batch_get_last_run_status(routine_ids).await
    }

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.inner.link_routine_run_to_job(run_id, job_id).await
    }

    async fn get_webhook_routine_by_path(
        &self,
        path: &str,
        user_id: Option<&str>,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.inner.get_webhook_routine_by_path(path, user_id).await
    }

    async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError> {
        self.inner.list_dispatched_routine_runs().await
    }
}

// ---------------------------------------------------------------------------
// ToolFailureStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl ToolFailureStore for ConcurrencyProbeDb {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        self.inner
            .record_tool_failure(tool_name, error_message)
            .await
    }

    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        self.inner.get_broken_tools(threshold).await
    }

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.inner.mark_tool_repaired(tool_name).await
    }

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.inner.increment_repair_attempts(tool_name).await
    }
}

// ---------------------------------------------------------------------------
// SettingsStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl SettingsStore for ConcurrencyProbeDb {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.inner.get_setting(user_id, key).await
    }

    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        self.inner.get_setting_full(user_id, key).await
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner.set_setting(user_id, key, value).await
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        self.inner.delete_setting(user_id, key).await
    }

    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        self.inner.list_settings(user_id).await
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.inner.get_all_settings(user_id).await
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.inner.set_all_settings(user_id, settings).await
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        self.inner.has_settings(user_id).await
    }
}

// ---------------------------------------------------------------------------
// UserStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl UserStore for ConcurrencyProbeDb {
    async fn create_user(&self, user: &UserRecord) -> Result<(), DatabaseError> {
        self.inner.create_user(user).await
    }

    async fn get_or_create_user(&self, user: UserRecord) -> Result<(), DatabaseError> {
        self.inner.get_or_create_user(user).await
    }

    async fn get_user(&self, id: &str) -> Result<Option<UserRecord>, DatabaseError> {
        self.inner.get_user(id).await
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<UserRecord>, DatabaseError> {
        self.inner.get_user_by_email(email).await
    }

    async fn list_users(&self, status: Option<&str>) -> Result<Vec<UserRecord>, DatabaseError> {
        self.inner.list_users(status).await
    }

    async fn update_user_status(&self, id: &str, status: &str) -> Result<(), DatabaseError> {
        self.inner.update_user_status(id, status).await
    }

    async fn update_user_role(&self, id: &str, role: &str) -> Result<(), DatabaseError> {
        self.inner.update_user_role(id, role).await
    }

    async fn update_user_profile(
        &self,
        id: &str,
        display_name: &str,
        metadata: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner
            .update_user_profile(id, display_name, metadata)
            .await
    }

    async fn record_login(&self, id: &str) -> Result<(), DatabaseError> {
        self.inner.record_login(id).await
    }

    async fn create_api_token(
        &self,
        user_id: &str,
        name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError> {
        self.inner
            .create_api_token(user_id, name, token_hash, token_prefix, expires_at)
            .await
    }

    async fn list_api_tokens(&self, user_id: &str) -> Result<Vec<ApiTokenRecord>, DatabaseError> {
        self.inner.list_api_tokens(user_id).await
    }

    async fn revoke_api_token(&self, token_id: Uuid, user_id: &str) -> Result<bool, DatabaseError> {
        self.inner.revoke_api_token(token_id, user_id).await
    }

    async fn authenticate_token(
        &self,
        token_hash: &[u8; 32],
    ) -> Result<Option<(ApiTokenRecord, UserRecord)>, DatabaseError> {
        self.inner.authenticate_token(token_hash).await
    }

    async fn record_token_usage(&self, token_id: Uuid) -> Result<(), DatabaseError> {
        self.inner.record_token_usage(token_id).await
    }

    async fn has_any_users(&self) -> Result<bool, DatabaseError> {
        self.inner.has_any_users().await
    }

    async fn delete_user(&self, id: &str) -> Result<bool, DatabaseError> {
        self.inner.delete_user(id).await
    }

    async fn user_usage_stats(
        &self,
        user_id: Option<&str>,
        since: DateTime<Utc>,
    ) -> Result<Vec<UserUsageStats>, DatabaseError> {
        self.inner.user_usage_stats(user_id, since).await
    }

    async fn user_summary_stats(
        &self,
        user_id: Option<&str>,
    ) -> Result<Vec<UserSummaryStats>, DatabaseError> {
        self.inner.user_summary_stats(user_id).await
    }

    async fn admin_usage_summary(
        &self,
        since: DateTime<Utc>,
    ) -> Result<AdminUsageSummary, DatabaseError> {
        self.inner.admin_usage_summary(since).await
    }

    async fn create_user_with_token(
        &self,
        user: &UserRecord,
        token_name: &str,
        token_hash: &[u8; 32],
        token_prefix: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<ApiTokenRecord, DatabaseError> {
        self.inner
            .create_user_with_token(user, token_name, token_hash, token_prefix, expires_at)
            .await
    }
}

// ---------------------------------------------------------------------------
// ChannelPairingStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl ironclaw::db::ChannelPairingStore for ConcurrencyProbeDb {
    async fn resolve_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<Option<UserId>, DatabaseError> {
        self.inner
            .resolve_channel_identity(channel, external_id)
            .await
    }

    async fn read_allow_from(&self, channel: &str) -> Result<Vec<String>, DatabaseError> {
        self.inner.read_allow_from(channel).await
    }

    async fn resolve_channel_external_id_for_owner(
        &self,
        channel: &str,
        owner_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        self.inner
            .resolve_channel_external_id_for_owner(channel, owner_id)
            .await
    }

    async fn upsert_pairing_request(
        &self,
        channel: &str,
        external_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<PairingRequestRecord, DatabaseError> {
        self.inner
            .upsert_pairing_request(channel, external_id, meta)
            .await
    }

    async fn approve_pairing(
        &self,
        channel: &str,
        code: &str,
        owner_id: &str,
    ) -> Result<PairingApprovalRecord, DatabaseError> {
        self.inner.approve_pairing(channel, code, owner_id).await
    }

    async fn revert_pairing_approval(
        &self,
        approval: &PairingApprovalRecord,
    ) -> Result<(), DatabaseError> {
        self.inner.revert_pairing_approval(approval).await
    }

    async fn list_pending_pairings(
        &self,
        channel: &str,
    ) -> Result<Vec<PairingRequestRecord>, DatabaseError> {
        self.inner.list_pending_pairings(channel).await
    }

    async fn remove_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
    ) -> Result<(), DatabaseError> {
        self.inner
            .remove_channel_identity(channel, external_id)
            .await
    }

    async fn create_channel_identity(
        &self,
        channel: &str,
        external_id: &str,
        owner_id: &str,
    ) -> Result<(), DatabaseError> {
        self.inner
            .create_channel_identity(channel, external_id, owner_id)
            .await
    }
}

// ---------------------------------------------------------------------------
// IdentityStore — forward everything
// ---------------------------------------------------------------------------

#[async_trait]
impl IdentityStore for ConcurrencyProbeDb {
    async fn get_identity_by_provider(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        self.inner
            .get_identity_by_provider(provider, provider_user_id)
            .await
    }

    async fn list_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserIdentityRecord>, DatabaseError> {
        self.inner.list_identities_for_user(user_id).await
    }

    async fn create_identity(&self, identity: &UserIdentityRecord) -> Result<(), DatabaseError> {
        self.inner.create_identity(identity).await
    }

    async fn update_identity_profile(
        &self,
        provider: &str,
        provider_user_id: &str,
        display_name: Option<&str>,
        avatar_url: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.inner
            .update_identity_profile(provider, provider_user_id, display_name, avatar_url)
            .await
    }

    async fn find_identity_by_verified_email(
        &self,
        email: &str,
    ) -> Result<Option<UserIdentityRecord>, DatabaseError> {
        self.inner.find_identity_by_verified_email(email).await
    }

    async fn create_user_with_identity(
        &self,
        user: &UserRecord,
        identity: &UserIdentityRecord,
    ) -> Result<(), DatabaseError> {
        self.inner.create_user_with_identity(user, identity).await
    }
}

// ---------------------------------------------------------------------------
// Database (supertrait) — forward migrations
// ---------------------------------------------------------------------------

#[async_trait]
impl Database for ConcurrencyProbeDb {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        self.inner.run_migrations().await
    }

    async fn migrate_default_owner(&self, owner_id: &str) -> Result<(), DatabaseError> {
        self.inner.migrate_default_owner(owner_id).await
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn make_test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
    use ironclaw::db::libsql::LibSqlBackend;

    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("test.db");
    let backend = LibSqlBackend::new_local(&db_path)
        .await
        .expect("LibSqlBackend::new_local");
    backend.run_migrations().await.expect("run_migrations");
    let db: Arc<dyn Database> = Arc::new(backend);
    (db, dir)
}

async fn make_run_context() -> LoopRunContext {
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::InMemoryRunProfileResolver,
    };

    let resolved = InMemoryRunProfileResolver::default()
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .unwrap();
    let scope = TurnScope::new(
        TenantId::new("tenant-parallel-read-test").unwrap(),
        None,
        None,
        ThreadId::new("thread-parallel-read-test").unwrap(),
    );
    LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved)
}

// ---------------------------------------------------------------------------
// The RED test
// ---------------------------------------------------------------------------

/// Seed all 7 identity files so `load_identity_candidates` has maximum work.
/// Files: 5 stable (SOUL, AGENTS, IDENTITY, TOOLS, BOOTSTRAP) + 2 personal.
async fn seed_all_identity_files(db: &Arc<dyn Database>) {
    let ws = Workspace::new_with_db("primary", Arc::clone(db));
    for path in &[
        paths::SOUL,
        paths::AGENTS,
        paths::IDENTITY,
        paths::TOOLS,
        paths::BOOTSTRAP,
        paths::USER,
        paths::ASSISTANT_DIRECTIVES,
    ] {
        ws.write(path, &format!("# {path}\n\ncontent for {path}"))
            .await
            .unwrap_or_else(|e| panic!("write {path}: {e}"));
    }
}

#[tokio::test]
async fn load_identity_candidates_is_parallel() {
    // 1. Real database with migrations applied.
    let (raw_db, _dir) = make_test_db().await;

    // 2. Seed all 7 identity files into the real DB.
    seed_all_identity_files(&raw_db).await;

    // 3. Wrap with the concurrency probe, sharing the counters externally.
    let current = Arc::new(AtomicUsize::new(0));
    let max = Arc::new(AtomicUsize::new(0));
    let probe = Arc::new(ConcurrencyProbeDb::new(
        Arc::clone(&raw_db),
        Arc::clone(&current),
        Arc::clone(&max),
    ));

    // 4. Build workspace → source backed by the probe.
    let ws = Arc::new(Workspace::new_with_db(
        "primary",
        probe as Arc<dyn Database>,
    ));
    let source = WorkspaceIdentityContextSource::new(ws);

    // 5. Run load_identity_candidates with personal context allowed so all 7
    //    files are attempted (5 stable + 2 personal).
    let mut ctx = make_run_context().await;
    ctx.resolved_run_profile.personal_context_policy = PersonalContextPolicy::Allowed;

    let candidates = source
        .load_identity_candidates(&ctx, PromptMode::TextOnly)
        .await
        .expect("load_identity_candidates must not fail");

    // Sanity: all 7 files should have been found.
    assert_eq!(
        candidates.len(),
        7,
        "expected 7 candidates (5 stable + 2 personal), got {}",
        candidates.len()
    );

    // 6. The key assertion: serial code produces max_observed == 1 because
    //    only one `get_document_by_path` is in-flight at a time.
    //    Parallel code would show max_observed >= 2.
    //    This test is RED (fails) until the implementation is parallelised.
    let max_observed = max.load(Ordering::SeqCst);
    assert!(
        max_observed >= 2,
        "load_identity_candidates is serial: only {max_observed} concurrent \
         get_document_by_path call(s) observed; expected >= 2. \
         Fix: run all candidate_for_path calls in parallel (e.g. futures::future::join_all)."
    );
}
