//! Filesystem-backed implementation of [`JobStore`].
//!
//! Routes agent-job persistence (jobs, actions, LLM calls, estimation
//! snapshots) through the unified
//! [`RootFilesystem`](ironclaw_filesystem::RootFilesystem) surface so the same
//! dispatch fabric used by `ironclaw_secrets`, `ironclaw_authorization`, and
//! [`FilesystemConversationStore`](super::filesystem_conversations::FilesystemConversationStore)
//! serves the job sub-trait too.
//!
//! Path layout (everything under the `/engine` virtual root):
//!
//! - `/engine/jobs/<job_id>` — agent-job record. Indexed: `user_id`,
//!   `status`, `source`, `category`, `created_at_ts`.
//! - `/engine/jobs/<job_id>/actions/<action_id>` — tool-action record.
//!   Indexed: `job_id`, `sequence` (i64), `tool_name`, `success` (bool),
//!   `created_at_ts`.
//! - `/engine/jobs/<job_id>/llm_calls/<call_id>` — recorded LLM call.
//! - `/engine/jobs/<job_id>/estimations/<estimation_id>` — estimation
//!   snapshot.
//!
//! Queries:
//!
//! - "list jobs for user X" -> `query("/engine/jobs",
//!   Filter::And(user_id, source))`.
//! - "stuck jobs" -> `query` with `Filter::Eq{status}`.
//! - "actions for a job" -> `query("/engine/jobs/<job_id>",
//!   Filter::Eq{job_id})`, sort by sequence in Rust.
//!
//! CAS: state transitions (`update_job_status`, `mark_job_stuck`) read the
//! current record's version and CAS-write on it. A per-job process-local
//! mutex matches the floor contract documented in
//! [`ironclaw_filesystem::CLAUDE.md`](../../../crates/ironclaw_filesystem/CLAUDE.md).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, Entry, Filter, IndexKey, IndexValue, Page, RecordKind, RecordVersion,
    RootFilesystem,
};
use ironclaw_host_api::VirtualPath;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::{ActionRecord, JobContext, JobState};
use crate::db::JobStore;
use crate::error::DatabaseError;
use crate::history::{AgentJobRecord, AgentJobSummary, LlmCallRecord};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredJob {
    id: Uuid,
    title: String,
    description: String,
    category: Option<String>,
    /// Snake-case (matches `JobState::Display` and the SQL backends).
    status: String,
    /// Provenance: `direct`, `system`, or `sandbox`.
    source: String,
    user_id: String,
    conversation_id: Option<Uuid>,
    budget_amount: Option<String>,
    budget_token: Option<String>,
    bid_amount: Option<String>,
    estimated_cost: Option<String>,
    estimated_time_secs: Option<i64>,
    actual_cost: String,
    repair_attempts: u32,
    max_tokens: u64,
    total_tokens_used: u64,
    failure_reason: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    stuck_since: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAction {
    id: Uuid,
    job_id: Uuid,
    sequence: u32,
    tool_name: String,
    input: serde_json::Value,
    output_raw: Option<String>,
    output_sanitized: Option<serde_json::Value>,
    sanitization_warnings: Vec<String>,
    cost: Option<String>,
    duration_ms: i64,
    success: bool,
    error: Option<String>,
    executed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredLlmCall {
    id: Uuid,
    job_id: Option<Uuid>,
    conversation_id: Option<Uuid>,
    provider: String,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cost: String,
    purpose: Option<String>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEstimation {
    id: Uuid,
    job_id: Uuid,
    category: String,
    tool_names: Vec<String>,
    estimated_cost: String,
    estimated_time_secs: i32,
    estimated_value: String,
    actual_cost: Option<String>,
    actual_time_secs: Option<i32>,
    actual_value: Option<String>,
    created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Filesystem-backed [`JobStore`].
pub struct FilesystemJobStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<F>,
}

impl<F> FilesystemJobStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }

    fn job_kind() -> RecordKind {
        RecordKind::new(KIND_JOB).expect("agent_job is a valid record-kind literal")
    }

    async fn read_job(
        &self,
        id: Uuid,
    ) -> Result<Option<(StoredJob, RecordVersion)>, DatabaseError> {
        let path = job_path(id)?;
        let Some(versioned) = self
            .filesystem
            .get(&path)
            .await
            .map_err(super::filesystem_conversations::fs_err_to_database)?
        else {
            return Ok(None);
        };
        let stored: StoredJob = serde_json::from_slice(&versioned.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        Ok(Some((stored, versioned.version)))
    }

    async fn write_job(
        &self,
        stored: &StoredJob,
        cas: CasExpectation,
    ) -> Result<RecordVersion, DatabaseError> {
        let path = job_path(stored.id)?;
        let body =
            serde_json::to_vec(stored).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let entry = build_job_entry(stored, body)?;
        self.filesystem
            .put(&path, entry, cas)
            .await
            .map_err(super::filesystem_conversations::fs_err_to_database)
    }
}

#[async_trait]
impl<F> JobStore for FilesystemJobStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        let lock = job_lock(ctx.job_id);
        let _guard = lock.lock().await;
        let existing = self.read_job(ctx.job_id).await?;
        let now = Utc::now();
        let stored = StoredJob {
            id: ctx.job_id,
            title: ctx.title.clone(),
            description: ctx.description.clone(),
            category: ctx.category.clone(),
            status: ctx.state.to_string(),
            source: existing
                .as_ref()
                .map(|(s, _)| s.source.clone())
                .unwrap_or_else(|| "direct".to_string()),
            user_id: ctx.user_id.clone(),
            conversation_id: ctx.conversation_id,
            budget_amount: ctx.budget.map(|d| d.to_string()),
            budget_token: ctx.budget_token.clone(),
            bid_amount: ctx.bid_amount.map(|d| d.to_string()),
            estimated_cost: ctx.estimated_cost.map(|d| d.to_string()),
            estimated_time_secs: ctx.estimated_duration.map(|d| d.as_secs() as i64),
            actual_cost: ctx.actual_cost.to_string(),
            repair_attempts: ctx.repair_attempts,
            max_tokens: ctx.max_tokens,
            total_tokens_used: ctx.total_tokens_used,
            failure_reason: existing
                .as_ref()
                .and_then(|(s, _)| s.failure_reason.clone()),
            created_at: ctx.created_at,
            started_at: ctx.started_at,
            completed_at: ctx.completed_at,
            stuck_since: existing.as_ref().and_then(|(s, _)| s.stuck_since),
        };
        // We deliberately overwrite (CAS::Any) because save_job is the catch-all
        // upsert mirroring the SQL ON CONFLICT pattern. Discrete transitions go
        // through dedicated methods that read-modify-write under a per-job
        // mutex (this fn already holds it).
        let _ = self.write_job(&stored, CasExpectation::Any).await?;
        // record the `now` to avoid an unused warning when the worker
        // pulls timestamps from `ctx` instead.
        let _ = now;
        Ok(())
    }

    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        let Some((stored, _)) = self.read_job(id).await? else {
            return Ok(None);
        };
        Ok(Some(stored_to_context(stored)?))
    }

    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let lock = job_lock(id);
        let _guard = lock.lock().await;
        let Some((mut stored, version)) = self.read_job(id).await? else {
            return Ok(());
        };
        stored.status = status.to_string();
        stored.failure_reason = failure_reason.map(str::to_string);
        let _ = self
            .write_job(&stored, CasExpectation::Version(version))
            .await?;
        Ok(())
    }

    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        let lock = job_lock(id);
        let _guard = lock.lock().await;
        let Some((mut stored, version)) = self.read_job(id).await? else {
            return Ok(());
        };
        stored.status = JobState::Stuck.to_string();
        stored.stuck_since = Some(Utc::now());
        let _ = self
            .write_job(&stored, CasExpectation::Version(version))
            .await?;
        Ok(())
    }

    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        let prefix = jobs_root()?;
        let filter = Filter::Eq {
            key: index_key(IDX_STATUS),
            value: IndexValue::Text(JobState::Stuck.to_string()),
        };
        let results = run_query(&self.filesystem, &prefix, &filter).await?;
        let mut ids = Vec::with_capacity(results.len());
        for v in results {
            if v.entry.kind.as_ref().map(|k| k.as_str()) != Some(KIND_JOB) {
                continue;
            }
            let stored: StoredJob = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            ids.push(stored.id);
        }
        Ok(ids)
    }

    async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        let prefix = jobs_root()?;
        let filter = Filter::Eq {
            key: index_key(IDX_SOURCE),
            value: IndexValue::Text("direct".to_string()),
        };
        list_agent_jobs_inner(&self.filesystem, &prefix, &filter).await
    }

    async fn list_agent_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        let prefix = jobs_root()?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key(IDX_SOURCE),
                value: IndexValue::Text("direct".to_string()),
            },
            Filter::Eq {
                key: index_key(IDX_USER_ID),
                value: IndexValue::Text(user_id.to_string()),
            },
        ]);
        list_agent_jobs_inner(&self.filesystem, &prefix, &filter).await
    }

    async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError> {
        let prefix = jobs_root()?;
        let filter = Filter::Eq {
            key: index_key(IDX_SOURCE),
            value: IndexValue::Text("direct".to_string()),
        };
        summary_inner(&self.filesystem, &prefix, &filter).await
    }

    async fn agent_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<AgentJobSummary, DatabaseError> {
        let prefix = jobs_root()?;
        let filter = Filter::And(vec![
            Filter::Eq {
                key: index_key(IDX_SOURCE),
                value: IndexValue::Text("direct".to_string()),
            },
            Filter::Eq {
                key: index_key(IDX_USER_ID),
                value: IndexValue::Text(user_id.to_string()),
            },
        ]);
        summary_inner(&self.filesystem, &prefix, &filter).await
    }

    async fn get_agent_job_failure_reason(
        &self,
        id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        Ok(self.read_job(id).await?.and_then(|(s, _)| s.failure_reason))
    }

    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError> {
        let stored = StoredAction {
            id: action.id,
            job_id,
            sequence: action.sequence,
            tool_name: action.tool_name.clone(),
            input: action.input.clone(),
            output_raw: action.output_raw.clone(),
            output_sanitized: action.output_sanitized.clone(),
            sanitization_warnings: action.sanitization_warnings.clone(),
            cost: action.cost.map(|d| d.to_string()),
            duration_ms: action.duration.as_millis() as i64,
            success: action.success,
            error: action.error.clone(),
            executed_at: action.executed_at,
        };
        let path = action_path(job_id, action.id)?;
        let body =
            serde_json::to_vec(&stored).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let entry = Entry::record(
            RecordKind::new(KIND_ACTION).expect("job_action is a valid record-kind literal"),
            &serde_json::Value::Null,
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let entry = Entry { body, ..entry }
            .with_indexed(index_key(IDX_JOB_ID), IndexValue::Text(job_id.to_string()))
            .with_indexed(
                index_key(IDX_SEQUENCE),
                IndexValue::I64(action.sequence as i64),
            )
            .with_indexed(
                index_key(IDX_TOOL_NAME),
                IndexValue::Text(action.tool_name.clone()),
            )
            .with_indexed(index_key(IDX_SUCCESS), IndexValue::Bool(action.success))
            .with_indexed(
                index_key(IDX_CREATED_AT_TS),
                IndexValue::I64(action.executed_at.timestamp_millis()),
            );
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map_err(super::filesystem_conversations::fs_err_to_database)?;
        Ok(())
    }

    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        let prefix = actions_root(job_id)?;
        let filter = Filter::Eq {
            key: index_key(IDX_JOB_ID),
            value: IndexValue::Text(job_id.to_string()),
        };
        let results = run_query(&self.filesystem, &prefix, &filter).await?;
        let mut actions = Vec::with_capacity(results.len());
        for v in results {
            if v.entry.kind.as_ref().map(|k| k.as_str()) != Some(KIND_ACTION) {
                continue;
            }
            let stored: StoredAction = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            if stored.job_id != job_id {
                continue;
            }
            actions.push(stored);
        }
        actions.sort_by_key(|a| a.sequence);
        Ok(actions.into_iter().map(stored_action_to_public).collect())
    }

    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        let id = Uuid::new_v4();
        let stored = StoredLlmCall {
            id,
            job_id: record.job_id,
            conversation_id: record.conversation_id,
            provider: record.provider.to_string(),
            model: record.model.to_string(),
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cost: record.cost.to_string(),
            purpose: record.purpose.map(str::to_string),
            created_at: Utc::now(),
        };
        // The legacy schema stores llm_calls under a top-level table, but a
        // job-scoped path keeps the filesystem layout hierarchical when a
        // job_id is present. When no job_id is attached we park the entry
        // under a synthetic "orphan" bucket so the entry still has a parent.
        let bucket = record.job_id.unwrap_or(Uuid::nil());
        let path = llm_call_path(bucket, id)?;
        let body =
            serde_json::to_vec(&stored).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let entry = Entry::record(
            RecordKind::new(KIND_LLM_CALL).expect("llm_call is a valid record-kind literal"),
            &serde_json::Value::Null,
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let mut entry = Entry { body, ..entry }
            .with_indexed(
                index_key(IDX_PROVIDER),
                IndexValue::Text(stored.provider.clone()),
            )
            .with_indexed(index_key(IDX_MODEL), IndexValue::Text(stored.model.clone()))
            .with_indexed(
                index_key(IDX_CREATED_AT_TS),
                IndexValue::I64(stored.created_at.timestamp_millis()),
            );
        if let Some(job_id) = record.job_id {
            entry = entry.with_indexed(index_key(IDX_JOB_ID), IndexValue::Text(job_id.to_string()));
        }
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map_err(super::filesystem_conversations::fs_err_to_database)?;
        Ok(id)
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
        let id = Uuid::new_v4();
        let stored = StoredEstimation {
            id,
            job_id,
            category: category.to_string(),
            tool_names: tool_names.to_vec(),
            estimated_cost: estimated_cost.to_string(),
            estimated_time_secs,
            estimated_value: estimated_value.to_string(),
            actual_cost: None,
            actual_time_secs: None,
            actual_value: None,
            created_at: Utc::now(),
        };
        let path = estimation_path(job_id, id)?;
        let body =
            serde_json::to_vec(&stored).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let entry = Entry::record(
            RecordKind::new(KIND_ESTIMATION)
                .expect("estimation_snapshot is a valid record-kind literal"),
            &serde_json::Value::Null,
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let entry = Entry { body, ..entry }
            .with_indexed(index_key(IDX_JOB_ID), IndexValue::Text(job_id.to_string()))
            .with_indexed(
                index_key(IDX_CATEGORY),
                IndexValue::Text(category.to_string()),
            );
        self.filesystem
            .put(&path, entry, CasExpectation::Any)
            .await
            .map_err(super::filesystem_conversations::fs_err_to_database)?;
        Ok(id)
    }

    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        // We have to find the estimation by id across all job buckets; the
        // filesystem layout puts estimations under their owning job. Query
        // the entire `/engine/jobs` subtree by id.
        let prefix = jobs_root()?;
        let filter = Filter::All; // narrow path-side check below
        let results = self
            .filesystem
            .query(&prefix, &filter, Page::new(0, Page::MAX_LIMIT))
            .await
            .map_err(super::filesystem_conversations::fs_err_to_database)?;
        for v in results {
            if v.entry.kind.as_ref().map(|k| k.as_str()) != Some(KIND_ESTIMATION) {
                continue;
            }
            let mut stored: StoredEstimation = serde_json::from_slice(&v.entry.body)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            if stored.id != id {
                continue;
            }
            stored.actual_cost = Some(actual_cost.to_string());
            stored.actual_time_secs = Some(actual_time_secs);
            stored.actual_value = actual_value.map(|d| d.to_string());
            let path = estimation_path(stored.job_id, stored.id)?;
            let body = serde_json::to_vec(&stored)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            let entry = Entry::record(
                RecordKind::new(KIND_ESTIMATION)
                    .expect("estimation_snapshot is a valid record-kind literal"),
                &serde_json::Value::Null,
            )
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            let entry = Entry { body, ..entry }
                .with_indexed(
                    index_key(IDX_JOB_ID),
                    IndexValue::Text(stored.job_id.to_string()),
                )
                .with_indexed(
                    index_key(IDX_CATEGORY),
                    IndexValue::Text(stored.category.clone()),
                );
            self.filesystem
                .put(&path, entry, CasExpectation::Version(v.version))
                .await
                .map_err(super::filesystem_conversations::fs_err_to_database)?;
            return Ok(());
        }
        Ok(())
    }

    async fn create_system_job(&self, user_id: &str, source: &str) -> Result<Uuid, DatabaseError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let stored = StoredJob {
            id,
            title: format!("System: {source}"),
            description: format!("System operation: {source}"),
            category: Some("system".to_string()),
            status: JobState::Completed.to_string(),
            source: "system".to_string(),
            user_id: user_id.to_string(),
            conversation_id: None,
            budget_amount: None,
            budget_token: None,
            bid_amount: None,
            estimated_cost: None,
            estimated_time_secs: None,
            actual_cost: "0".to_string(),
            repair_attempts: 0,
            max_tokens: 0,
            total_tokens_used: 0,
            failure_reason: None,
            created_at: now,
            started_at: Some(now),
            completed_at: Some(now),
            stuck_since: None,
        };
        let _ = self.write_job(&stored, CasExpectation::Absent).await?;
        Ok(id)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn stored_to_context(stored: StoredJob) -> Result<JobContext, DatabaseError> {
    let state = parse_state(&stored.status);
    let budget = parse_decimal(stored.budget_amount.as_deref())?;
    let bid_amount = parse_decimal(stored.bid_amount.as_deref())?;
    let estimated_cost = parse_decimal(stored.estimated_cost.as_deref())?;
    let actual_cost = parse_decimal(Some(&stored.actual_cost))?.unwrap_or(Decimal::ZERO);

    Ok(JobContext {
        job_id: stored.id,
        state,
        user_id: stored.user_id,
        requester_id: None,
        conversation_id: stored.conversation_id,
        title: stored.title,
        description: stored.description,
        category: stored.category,
        budget,
        budget_token: stored.budget_token,
        bid_amount,
        estimated_cost,
        estimated_duration: stored
            .estimated_time_secs
            .map(|s| std::time::Duration::from_secs(s.max(0) as u64)),
        actual_cost,
        max_tokens: stored.max_tokens,
        total_tokens_used: stored.total_tokens_used,
        repair_attempts: stored.repair_attempts,
        created_at: stored.created_at,
        started_at: stored.started_at,
        completed_at: stored.completed_at,
        transitions: Vec::new(),
        metadata: serde_json::Value::Null,
        extra_env: Arc::new(HashMap::new()),
        http_interceptor: None,
        tool_output_stash: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        user_timezone: "UTC".to_string(),
        approval_context: None,
    })
}

fn parse_state(s: &str) -> JobState {
    match s {
        "pending" => JobState::Pending,
        "in_progress" => JobState::InProgress,
        "completed" => JobState::Completed,
        "submitted" => JobState::Submitted,
        "accepted" => JobState::Accepted,
        "failed" => JobState::Failed,
        "stuck" => JobState::Stuck,
        "cancelled" => JobState::Cancelled,
        _ => JobState::Pending,
    }
}

fn parse_decimal(s: Option<&str>) -> Result<Option<Decimal>, DatabaseError> {
    let Some(s) = s else { return Ok(None) };
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<Decimal>()
        .map(Some)
        .map_err(|e| DatabaseError::Serialization(e.to_string()))
}

fn stored_action_to_public(stored: StoredAction) -> ActionRecord {
    ActionRecord {
        id: stored.id,
        sequence: stored.sequence,
        tool_name: stored.tool_name,
        input: stored.input,
        output_raw: stored.output_raw,
        output_sanitized: stored.output_sanitized,
        sanitization_warnings: stored.sanitization_warnings,
        cost: stored
            .cost
            .as_deref()
            .and_then(|s| s.parse::<Decimal>().ok()),
        duration: std::time::Duration::from_millis(stored.duration_ms.max(0) as u64),
        success: stored.success,
        error: stored.error,
        executed_at: stored.executed_at,
    }
}

fn build_job_entry(stored: &StoredJob, body: Vec<u8>) -> Result<Entry, DatabaseError> {
    let entry = Entry::record(
        FilesystemJobStore::<ironclaw_filesystem::InMemoryBackend>::job_kind(),
        &serde_json::Value::Null,
    )
    .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let mut entry = Entry { body, ..entry }
        .with_indexed(
            index_key(IDX_USER_ID),
            IndexValue::Text(stored.user_id.clone()),
        )
        .with_indexed(
            index_key(IDX_STATUS),
            IndexValue::Text(stored.status.clone()),
        )
        .with_indexed(
            index_key(IDX_SOURCE),
            IndexValue::Text(stored.source.clone()),
        )
        .with_indexed(
            index_key(IDX_CREATED_AT_TS),
            IndexValue::I64(stored.created_at.timestamp_millis()),
        );
    if let Some(category) = &stored.category {
        entry = entry.with_indexed(index_key(IDX_CATEGORY), IndexValue::Text(category.clone()));
    }
    Ok(entry)
}

async fn run_query<F: RootFilesystem>(
    filesystem: &Arc<F>,
    prefix: &VirtualPath,
    filter: &Filter,
) -> Result<Vec<ironclaw_filesystem::VersionedEntry>, DatabaseError> {
    match filesystem
        .query(prefix, filter, Page::new(0, Page::MAX_LIMIT))
        .await
    {
        Ok(r) => Ok(r),
        Err(error) if super::filesystem_conversations::is_not_found(&error) => Ok(Vec::new()),
        Err(error) => Err(super::filesystem_conversations::fs_err_to_database(error)),
    }
}

async fn list_agent_jobs_inner<F: RootFilesystem>(
    filesystem: &Arc<F>,
    prefix: &VirtualPath,
    filter: &Filter,
) -> Result<Vec<AgentJobRecord>, DatabaseError> {
    let results = run_query(filesystem, prefix, filter).await?;
    let mut jobs: Vec<StoredJob> = Vec::with_capacity(results.len());
    for v in results {
        if v.entry.kind.as_ref().map(|k| k.as_str()) != Some(KIND_JOB) {
            continue;
        }
        let stored: StoredJob = serde_json::from_slice(&v.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        jobs.push(stored);
    }
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(jobs
        .into_iter()
        .map(|s| AgentJobRecord {
            id: s.id,
            title: s.title,
            status: s.status,
            user_id: s.user_id,
            failure_reason: s.failure_reason,
            created_at: s.created_at,
            started_at: s.started_at,
            completed_at: s.completed_at,
        })
        .collect())
}

async fn summary_inner<F: RootFilesystem>(
    filesystem: &Arc<F>,
    prefix: &VirtualPath,
    filter: &Filter,
) -> Result<AgentJobSummary, DatabaseError> {
    let results = run_query(filesystem, prefix, filter).await?;
    let mut summary = AgentJobSummary::default();
    for v in results {
        if v.entry.kind.as_ref().map(|k| k.as_str()) != Some(KIND_JOB) {
            continue;
        }
        let stored: StoredJob = serde_json::from_slice(&v.entry.body)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        summary.add_count(&stored.status, 1);
    }
    Ok(summary)
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const ROOT: &str = "/engine/jobs";

fn jobs_root() -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(ROOT).map_err(|e| DatabaseError::Query(e.to_string()))
}

fn job_path(id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("{ROOT}/{id}")).map_err(|e| DatabaseError::Query(e.to_string()))
}

fn actions_root(job_id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("{ROOT}/{job_id}/actions"))
        .map_err(|e| DatabaseError::Query(e.to_string()))
}

fn action_path(job_id: Uuid, action_id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("{ROOT}/{job_id}/actions/{action_id}"))
        .map_err(|e| DatabaseError::Query(e.to_string()))
}

fn llm_call_path(job_id: Uuid, call_id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("{ROOT}/{job_id}/llm_calls/{call_id}"))
        .map_err(|e| DatabaseError::Query(e.to_string()))
}

fn estimation_path(job_id: Uuid, est_id: Uuid) -> Result<VirtualPath, DatabaseError> {
    VirtualPath::new(format!("{ROOT}/{job_id}/estimations/{est_id}"))
        .map_err(|e| DatabaseError::Query(e.to_string()))
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub(crate) const KIND_JOB: &str = "agent_job";
pub(crate) const KIND_ACTION: &str = "job_action";
pub(crate) const KIND_LLM_CALL: &str = "llm_call";
pub(crate) const KIND_ESTIMATION: &str = "estimation_snapshot";

pub(crate) const IDX_USER_ID: &str = "user_id";
pub(crate) const IDX_STATUS: &str = "status";
pub(crate) const IDX_SOURCE: &str = "source";
pub(crate) const IDX_CATEGORY: &str = "category";
pub(crate) const IDX_CREATED_AT_TS: &str = "created_at_ts";
pub(crate) const IDX_JOB_ID: &str = "job_id";
pub(crate) const IDX_SEQUENCE: &str = "sequence_num";
pub(crate) const IDX_TOOL_NAME: &str = "tool_name";
pub(crate) const IDX_SUCCESS: &str = "success";
pub(crate) const IDX_PROVIDER: &str = "provider";
pub(crate) const IDX_MODEL: &str = "model";

pub(crate) fn index_key(name: &'static str) -> IndexKey {
    IndexKey::new(name).expect("index key literal is valid")
}

// ---------------------------------------------------------------------------
// Locks
// ---------------------------------------------------------------------------

type RecordLock = Arc<tokio::sync::Mutex<()>>;

static FILESYSTEM_RECORD_LOCKS: OnceLock<Mutex<HashMap<String, RecordLock>>> = OnceLock::new();

fn record_lock(key: String) -> RecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = lock_or_recover(locks);
    Arc::clone(
        guard
            .entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

fn lock_or_recover<T>(mutex: &Mutex<HashMap<String, T>>) -> MutexGuard<'_, HashMap<String, T>> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn job_lock(id: Uuid) -> RecordLock {
    record_lock(format!("job|{id}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use rust_decimal_macros::dec;
    use std::time::Duration;

    fn new_store() -> FilesystemJobStore<InMemoryBackend> {
        FilesystemJobStore::new(Arc::new(InMemoryBackend::new()))
    }

    fn sample_ctx(user: &str) -> JobContext {
        JobContext::with_user(user, "title", "description")
    }

    #[tokio::test]
    async fn save_and_get_job_round_trip() {
        let store = new_store();
        let ctx = sample_ctx("user-a");
        let id = ctx.job_id;
        store.save_job(&ctx).await.unwrap();
        let restored = store.get_job(id).await.unwrap().unwrap();
        assert_eq!(restored.job_id, id);
        assert_eq!(restored.user_id, "user-a");
        assert_eq!(restored.title, "title");
    }

    #[tokio::test]
    async fn update_status_and_mark_stuck_use_cas() {
        let store = new_store();
        let ctx = sample_ctx("user-a");
        let id = ctx.job_id;
        store.save_job(&ctx).await.unwrap();
        store
            .update_job_status(id, JobState::InProgress, None)
            .await
            .unwrap();
        let after = store.get_job(id).await.unwrap().unwrap();
        assert_eq!(after.state, JobState::InProgress);
        store.mark_job_stuck(id).await.unwrap();
        let stuck = store.get_stuck_jobs().await.unwrap();
        assert!(stuck.contains(&id));
    }

    #[tokio::test]
    async fn list_and_summary_filter_to_direct_source() {
        let store = new_store();
        let direct = sample_ctx("user-a");
        let direct_id = direct.job_id;
        store.save_job(&direct).await.unwrap();
        let system_id = store
            .create_system_job("user-a", "test-source")
            .await
            .unwrap();
        let agent = store.list_agent_jobs().await.unwrap();
        let ids: Vec<_> = agent.iter().map(|r| r.id).collect();
        assert!(ids.contains(&direct_id));
        assert!(!ids.contains(&system_id));
        let summary = store.agent_job_summary().await.unwrap();
        assert_eq!(summary.total, 1);
    }

    #[tokio::test]
    async fn list_agent_jobs_for_user_filters_user() {
        let store = new_store();
        let a = sample_ctx("user-a");
        let b = sample_ctx("user-b");
        let a_id = a.job_id;
        store.save_job(&a).await.unwrap();
        store.save_job(&b).await.unwrap();
        let only_a = store.list_agent_jobs_for_user("user-a").await.unwrap();
        let ids: Vec<_> = only_a.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![a_id]);
    }

    #[tokio::test]
    async fn get_agent_job_failure_reason_round_trip() {
        let store = new_store();
        let ctx = sample_ctx("user-a");
        let id = ctx.job_id;
        store.save_job(&ctx).await.unwrap();
        assert!(
            store
                .get_agent_job_failure_reason(id)
                .await
                .unwrap()
                .is_none()
        );
        store
            .update_job_status(id, JobState::Failed, Some("boom"))
            .await
            .unwrap();
        assert_eq!(
            store
                .get_agent_job_failure_reason(id)
                .await
                .unwrap()
                .as_deref(),
            Some("boom")
        );
    }

    #[tokio::test]
    async fn save_action_and_query_back() {
        let store = new_store();
        let ctx = sample_ctx("user-a");
        let job_id = ctx.job_id;
        store.save_job(&ctx).await.unwrap();
        for i in 0..3u32 {
            let action = ActionRecord::new(i, "echo", serde_json::json!({"i": i})).succeed(
                Some("ok".to_string()),
                serde_json::json!({"out": i}),
                Duration::from_millis(5),
            );
            store.save_action(job_id, &action).await.unwrap();
        }
        let actions = store.get_job_actions(job_id).await.unwrap();
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].sequence, 0);
        assert_eq!(actions[2].sequence, 2);
        assert!(actions.iter().all(|a| a.success));
    }

    #[tokio::test]
    async fn record_llm_call_returns_uuid() {
        let store = new_store();
        let id = store
            .record_llm_call(&LlmCallRecord {
                job_id: None,
                conversation_id: None,
                provider: "openai",
                model: "gpt-4o",
                input_tokens: 10,
                output_tokens: 20,
                cost: dec!(0.001),
                purpose: Some("test"),
            })
            .await
            .unwrap();
        assert_ne!(id, Uuid::nil());
    }

    #[tokio::test]
    async fn estimation_snapshot_actuals_round_trip() {
        let store = new_store();
        let job = Uuid::new_v4();
        let id = store
            .save_estimation_snapshot(
                job,
                "research",
                &["echo".to_string(), "http".to_string()],
                dec!(0.05),
                30,
                dec!(0.10),
            )
            .await
            .unwrap();
        store
            .update_estimation_actuals(id, dec!(0.04), 25, Some(dec!(0.09)))
            .await
            .unwrap();
        // Re-querying the same entry should now have actuals set.
        let prefix = jobs_root().unwrap();
        let results = store
            .filesystem
            .query(&prefix, &Filter::All, Page::new(0, Page::MAX_LIMIT))
            .await
            .unwrap();
        let stored = results
            .iter()
            .filter(|v| v.entry.kind.as_ref().map(|k| k.as_str()) == Some(KIND_ESTIMATION))
            .find_map(|v| serde_json::from_slice::<StoredEstimation>(&v.entry.body).ok())
            .expect("estimation persisted");
        assert_eq!(stored.actual_cost.as_deref(), Some("0.04"));
        assert_eq!(stored.actual_time_secs, Some(25));
    }

    #[tokio::test]
    async fn create_system_job_writes_completed_row() {
        let store = new_store();
        let id = store.create_system_job("user-a", "dispatch").await.unwrap();
        let stored = store.get_job(id).await.unwrap().unwrap();
        assert_eq!(stored.state, JobState::Completed);
        assert_eq!(stored.category.as_deref(), Some("system"));
        // System jobs must not appear in list_agent_jobs() (filtered by source).
        let listed = store.list_agent_jobs().await.unwrap();
        assert!(!listed.iter().any(|r| r.id == id));
    }

    #[tokio::test]
    async fn update_status_on_missing_job_is_noop() {
        let store = new_store();
        let missing = Uuid::new_v4();
        store
            .update_job_status(missing, JobState::Failed, Some("nope"))
            .await
            .unwrap();
        assert!(store.get_job(missing).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn agent_job_summary_for_user_segregates() {
        let store = new_store();
        let a = sample_ctx("user-a");
        let b = sample_ctx("user-b");
        store.save_job(&a).await.unwrap();
        store.save_job(&b).await.unwrap();
        let only_a = store.agent_job_summary_for_user("user-a").await.unwrap();
        assert_eq!(only_a.total, 1);
        let total = store.agent_job_summary().await.unwrap();
        assert_eq!(total.total, 2);
    }
}
