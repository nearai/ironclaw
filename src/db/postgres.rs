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
    ConversationStore, Database, JobStore, RoutineStore, SandboxStore, SettingsStore,
    ToolFailureStore, WorkspaceStore, structured,
    structured::{
        Aggregation, CollectionSchema, Filter, Record, StructuredStore,
    },
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

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .ensure_conversation(id, channel, user_id, thread_id)
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

    async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError> {
        self.store.agent_job_summary().await
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

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.store.link_routine_run_to_job(run_id, job_id).await
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
        let field = &filter.field;
        structured::validate_field_name(field)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match filter.op {
            structured::FilterOp::IsNull => {
                clauses.push(format!("data->>'{field}' IS NULL"));
            }
            structured::FilterOp::IsNotNull => {
                clauses.push(format!("data->>'{field}' IS NOT NULL"));
            }
            structured::FilterOp::Eq => {
                clauses.push(format!("data->>'{field}' = ${idx}"));
                let val = json_value_to_text_string(&filter.value);
                params.push(Box::new(val));
                idx += 1;
            }
            structured::FilterOp::Neq => {
                clauses.push(format!("data->>'{field}' != ${idx}"));
                let val = json_value_to_text_string(&filter.value);
                params.push(Box::new(val));
                idx += 1;
            }
            structured::FilterOp::Gt => {
                clauses.push(format!("data->>'{field}' > ${idx}"));
                let val = json_value_to_text_string(&filter.value);
                params.push(Box::new(val));
                idx += 1;
            }
            structured::FilterOp::Gte => {
                clauses.push(format!("data->>'{field}' >= ${idx}"));
                let val = json_value_to_text_string(&filter.value);
                params.push(Box::new(val));
                idx += 1;
            }
            structured::FilterOp::Lt => {
                clauses.push(format!("data->>'{field}' < ${idx}"));
                let val = json_value_to_text_string(&filter.value);
                params.push(Box::new(val));
                idx += 1;
            }
            structured::FilterOp::Lte => {
                clauses.push(format!("data->>'{field}' <= ${idx}"));
                let val = json_value_to_text_string(&filter.value);
                params.push(Box::new(val));
                idx += 1;
            }
            structured::FilterOp::Between => {
                let arr = filter.value.as_array().ok_or_else(|| {
                    DatabaseError::Query(
                        "Between filter requires an array of [lo, hi]".to_string(),
                    )
                })?;
                if arr.len() != 2 {
                    return Err(DatabaseError::Query(
                        "Between filter requires exactly 2 elements".to_string(),
                    ));
                }
                clauses.push(format!(
                    "data->>'{field}' BETWEEN ${idx} AND ${}",
                    idx + 1
                ));
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
                    clauses.push(format!(
                        "data->>'{field}' IN ({})",
                        placeholders.join(", ")
                    ));
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
        serde_json::from_value(schema_json)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))
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

    async fn drop_collection(
        &self,
        user_id: &str,
        collection: &str,
    ) -> Result<(), DatabaseError> {
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

    async fn get_record(
        &self,
        user_id: &str,
        record_id: Uuid,
    ) -> Result<Record, DatabaseError> {
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
        // Fetch existing record to get its collection and current data.
        let existing = self.get_record(user_id, record_id).await?;
        let schema = self
            .get_collection_schema(user_id, &existing.collection)
            .await?;

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

        let conn = self.store.conn().await?;
        let n = conn
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
        Ok(())
    }

    async fn delete_record(
        &self,
        user_id: &str,
        record_id: Uuid,
    ) -> Result<(), DatabaseError> {
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
        let capped_limit = limit.min(1000) as i64;

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
                sql.push_str(&format!(" ORDER BY data->>'{field}'"));
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
        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync)).collect();

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
                let field = aggregation.field.as_deref().ok_or_else(|| {
                    DatabaseError::Query("Sum requires a field".to_string())
                })?;
                format!("SUM((data->>'{field}')::numeric)")
            }
            structured::AggOp::Avg => {
                let field = aggregation.field.as_deref().ok_or_else(|| {
                    DatabaseError::Query("Avg requires a field".to_string())
                })?;
                format!("AVG((data->>'{field}')::numeric)")
            }
            structured::AggOp::Min => {
                let field = aggregation.field.as_deref().ok_or_else(|| {
                    DatabaseError::Query("Min requires a field".to_string())
                })?;
                format!("MIN(data->>'{field}')")
            }
            structured::AggOp::Max => {
                let field = aggregation.field.as_deref().ok_or_else(|| {
                    DatabaseError::Query("Max requires a field".to_string())
                })?;
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

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync)).collect();

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
            let row = rows.first().ok_or_else(|| {
                DatabaseError::Query("Aggregation returned no rows".to_string())
            })?;
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
                        DatabaseError::Query(format!(
                            "Cannot convert aggregate result {d} to f64"
                        ))
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
