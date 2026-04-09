//! Compile-time tenant isolation.
//!
//! Provides three database access tiers:
//!
//! - **[`TenantScope`]** (default): All operations are bound to a single user.
//!   ID-based lookups return `None` if the resource doesn't belong to this user.
//!   This is the only way handler code should access the database.
//!
//! - **[`SystemScope`]**: Cross-tenant access for system-level operations
//!   (heartbeat, routine engine, self-repair). Must be obtained explicitly via
//!   [`AgentDeps::system_store()`](crate::agent::AgentDeps::system_store).
//!   Not for human actors.
//!
//! - **[`AdminScope`]**: Human admin operations (user management). Requires
//!   `UserRole::Admin`. Constructable only via [`AdminScope::new`].
//!
//! [`TenantCtx`] bundles a `TenantScope` with workspace, cost guard, and
//! per-tenant rate limiting. Constructed once per request at the entry point
//! where a `user_id` becomes known.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio::sync::{Semaphore, SemaphorePermit};
use uuid::Uuid;

use crate::agent::BrokenTool;
use crate::agent::cost_guard::{CostGuard, CostLimitExceeded};
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::context::{ActionRecord, JobContext, JobState};
use crate::db::Database;
use crate::error::DatabaseError;
use crate::history::{
    AgentJobRecord, AgentJobSummary, ConversationMessage, ConversationSummary, LlmCallRecord,
    SandboxJobRecord, SandboxJobSummary, SettingRow,
};
use crate::ownership::Owned;
use crate::workspace::Workspace;

// ---------------------------------------------------------------------------
// TenantScope — scoped database access (default tier)
// ---------------------------------------------------------------------------

/// Scoped database view. All operations are bound to a single user.
///
/// This is the **only** way handler code should access the database.
/// ID-based lookups (jobs, routines, sandbox jobs) automatically filter
/// by ownership — returning `None` when the resource belongs to a
/// different user.
#[derive(Clone)]
pub struct TenantScope {
    identity: crate::ownership::Identity,
    inner: Arc<dyn Database>,
}

impl TenantScope {
    /// Construct from a resolved `Identity` (preferred).
    pub fn with_identity(identity: crate::ownership::Identity, db: Arc<dyn Database>) -> Self {
        Self {
            identity,
            inner: db,
        }
    }

    /// Bridge constructor for call sites not yet migrated to `Identity`.
    /// Creates a Member-role identity from a raw user_id string.
    pub fn new(user_id: impl Into<String>, db: Arc<dyn Database>) -> Self {
        use crate::ownership::{Identity, OwnerId, UserRole};
        Self::with_identity(
            Identity::new(OwnerId::from(user_id.into()), UserRole::Member),
            db,
        )
    }

    pub fn identity(&self) -> &crate::ownership::Identity {
        &self.identity
    }

    pub fn user_id(&self) -> &str {
        self.identity.owner_id.as_str()
    }

    // === Jobs ===

    pub async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        self.inner
            .list_agent_jobs_for_user(self.identity.owner_id.as_str())
            .await
    }

    pub async fn agent_job_summary(&self) -> Result<AgentJobSummary, DatabaseError> {
        self.inner
            .agent_job_summary_for_user(self.identity.owner_id.as_str())
            .await
    }

    /// Fetch a job by ID, returning `None` if it doesn't belong to this user.
    pub async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        match self.inner.get_job(id).await? {
            Some(ctx) if ctx.is_owned_by(self.identity.owner_id.as_str()) => Ok(Some(ctx)),
            _ => Ok(None),
        }
    }

    pub async fn get_agent_job_failure_reason(
        &self,
        id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        // Verify ownership first
        if self.get_job(id).await?.is_none() {
            return Ok(None);
        }
        self.inner.get_agent_job_failure_reason(id).await
    }

    pub async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        // Verify ownership before mutating
        if self.get_job(id).await?.is_none() {
            return Err(DatabaseError::NotFound {
                entity: "job".to_string(),
                id: id.to_string(),
            });
        }
        self.inner
            .update_job_status(id, status, failure_reason)
            .await
    }

    // === Sandbox jobs ===

    pub async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.inner
            .list_sandbox_jobs_for_user(self.identity.owner_id.as_str())
            .await
    }

    pub async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        self.inner
            .sandbox_job_summary_for_user(self.identity.owner_id.as_str())
            .await
    }

    /// Fetch a sandbox job by ID, returning `None` if it doesn't belong to this user.
    pub async fn get_sandbox_job(
        &self,
        id: Uuid,
    ) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        match self.inner.get_sandbox_job(id).await? {
            Some(job) if job.is_owned_by(self.identity.owner_id.as_str()) => Ok(Some(job)),
            _ => Ok(None),
        }
    }

    pub async fn sandbox_job_belongs_to_user(&self, job_id: Uuid) -> Result<bool, DatabaseError> {
        self.inner
            .sandbox_job_belongs_to_user(job_id, self.identity.owner_id.as_str())
            .await
    }

    // === Routines ===

    pub async fn list_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner
            .list_routines(self.identity.owner_id.as_str())
            .await
    }

    pub async fn get_routine_by_name(&self, name: &str) -> Result<Option<Routine>, DatabaseError> {
        self.inner
            .get_routine_by_name(self.identity.owner_id.as_str(), name)
            .await
    }

    /// Fetch a routine by ID, returning `None` if it doesn't belong to this user.
    pub async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        match self.inner.get_routine(id).await? {
            Some(r) if r.is_owned_by(self.identity.owner_id.as_str()) => Ok(Some(r)),
            _ => Ok(None),
        }
    }

    pub async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        debug_assert_eq!(
            routine.user_id,
            self.identity.owner_id.as_str(),
            "routine.user_id must match TenantScope user"
        );
        self.inner.create_routine(routine).await
    }

    pub async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        // Verify ownership
        if self.get_routine(routine.id).await?.is_none() {
            return Err(DatabaseError::NotFound {
                entity: "routine".to_string(),
                id: routine.id.to_string(),
            });
        }
        self.inner.update_routine(routine).await
    }

    pub async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        // Verify ownership
        if self.get_routine(id).await?.is_none() {
            return Err(DatabaseError::NotFound {
                entity: "routine".to_string(),
                id: id.to_string(),
            });
        }
        self.inner.delete_routine(id).await
    }

    /// List routine runs, verifying the routine belongs to this user.
    pub async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        // Verify routine ownership first
        if self.get_routine(routine_id).await?.is_none() {
            return Err(DatabaseError::NotFound {
                entity: "routine".to_string(),
                id: routine_id.to_string(),
            });
        }
        self.inner.list_routine_runs(routine_id, limit).await
    }

    pub async fn get_webhook_routine_by_path(
        &self,
        path: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.inner
            .get_webhook_routine_by_path(path, Some(self.identity.owner_id.as_str()))
            .await
    }

    // === LLM call recording ===

    /// Record an LLM call to the database for persistent usage tracking.
    pub async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        self.inner.record_llm_call(record).await
    }

    // === Settings ===

    pub async fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.inner
            .get_setting(self.identity.owner_id.as_str(), key)
            .await
    }

    pub async fn get_setting_full(&self, key: &str) -> Result<Option<SettingRow>, DatabaseError> {
        self.inner
            .get_setting_full(self.identity.owner_id.as_str(), key)
            .await
    }

    pub async fn set_setting(
        &self,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner
            .set_setting(self.identity.owner_id.as_str(), key, value)
            .await
    }

    pub async fn delete_setting(&self, key: &str) -> Result<bool, DatabaseError> {
        self.inner
            .delete_setting(self.identity.owner_id.as_str(), key)
            .await
    }

    pub async fn list_settings(&self) -> Result<Vec<SettingRow>, DatabaseError> {
        self.inner
            .list_settings(self.identity.owner_id.as_str())
            .await
    }

    pub async fn get_all_settings(
        &self,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.inner
            .get_all_settings(self.identity.owner_id.as_str())
            .await
    }

    pub async fn set_all_settings(
        &self,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.inner
            .set_all_settings(self.identity.owner_id.as_str(), settings)
            .await
    }

    pub async fn has_settings(&self) -> Result<bool, DatabaseError> {
        self.inner
            .has_settings(self.identity.owner_id.as_str())
            .await
    }

    // === Conversations ===

    pub async fn create_conversation(
        &self,
        channel: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .create_conversation(channel, self.identity.owner_id.as_str(), thread_id)
            .await
    }

    pub async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        thread_id: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        self.inner
            .ensure_conversation(
                id,
                channel,
                self.identity.owner_id.as_str(),
                thread_id,
                Some(channel),
            )
            .await
    }

    pub async fn list_conversations_with_preview(
        &self,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.inner
            .list_conversations_with_preview(self.identity.owner_id.as_str(), channel, limit)
            .await
    }

    pub async fn list_conversations_all_channels(
        &self,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.inner
            .list_conversations_all_channels(self.identity.owner_id.as_str(), limit)
            .await
    }

    pub async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .get_or_create_routine_conversation(
                routine_id,
                routine_name,
                self.identity.owner_id.as_str(),
            )
            .await
    }

    pub async fn get_or_create_heartbeat_conversation(&self) -> Result<Uuid, DatabaseError> {
        self.inner
            .get_or_create_heartbeat_conversation(self.identity.owner_id.as_str())
            .await
    }

    pub async fn get_or_create_assistant_conversation(
        &self,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .get_or_create_assistant_conversation(self.identity.owner_id.as_str(), channel)
            .await
    }

    pub async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        self.inner
            .conversation_belongs_to_user(conversation_id, self.identity.owner_id.as_str())
            .await
    }

    /// Add a message to a conversation owned by this tenant.
    ///
    /// Returns `NotFound` if the conversation does not belong to this user.
    pub async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        if !self.conversation_belongs_to_user(conversation_id).await? {
            return Err(DatabaseError::NotFound {
                entity: "conversation".to_string(),
                id: conversation_id.to_string(),
            });
        }
        self.inner
            .add_conversation_message(conversation_id, role, content)
            .await
    }

    /// Touch a conversation timestamp. Returns `NotFound` if not owned by this user.
    pub async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        if !self.conversation_belongs_to_user(id).await? {
            return Err(DatabaseError::NotFound {
                entity: "conversation".to_string(),
                id: id.to_string(),
            });
        }
        self.inner.touch_conversation(id).await
    }

    /// List messages in a conversation. Returns `NotFound` if not owned by this user.
    pub async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        if !self.conversation_belongs_to_user(conversation_id).await? {
            return Err(DatabaseError::NotFound {
                entity: "conversation".to_string(),
                id: conversation_id.to_string(),
            });
        }
        self.inner.list_conversation_messages(conversation_id).await
    }

    /// Paginated message listing. Returns `NotFound` if not owned by this user.
    pub async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        if !self.conversation_belongs_to_user(conversation_id).await? {
            return Err(DatabaseError::NotFound {
                entity: "conversation".to_string(),
                id: conversation_id.to_string(),
            });
        }
        self.inner
            .list_conversation_messages_paginated(conversation_id, before, limit)
            .await
    }

    pub async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .create_conversation_with_metadata(channel, self.identity.owner_id.as_str(), metadata)
            .await
    }

    /// Update metadata on a conversation. Returns `NotFound` if not owned by this user.
    pub async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        if !self.conversation_belongs_to_user(id).await? {
            return Err(DatabaseError::NotFound {
                entity: "conversation".to_string(),
                id: id.to_string(),
            });
        }
        self.inner
            .update_conversation_metadata_field(id, key, value)
            .await
    }

    /// Get conversation metadata. Returns `NotFound` if not owned by this user.
    pub async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        if !self.conversation_belongs_to_user(id).await? {
            return Err(DatabaseError::NotFound {
                entity: "conversation".to_string(),
                id: id.to_string(),
            });
        }
        self.inner.get_conversation_metadata(id).await
    }
}

// ---------------------------------------------------------------------------
// SystemScope — cross-tenant access for system processes only
// ---------------------------------------------------------------------------

/// Cross-tenant database access for system-level operations (not human actors).
///
/// **Not** available through [`TenantCtx`] — must be obtained explicitly via
/// [`AgentDeps::system_store()`](crate::agent::AgentDeps::system_store).
///
/// Used by: heartbeat enumeration, routine engine scheduling, self-repair,
/// scheduler job persistence, worker status updates.
#[derive(Clone)]
pub struct SystemScope {
    inner: Arc<dyn Database>,
}

impl SystemScope {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { inner: db }
    }

    /// Construct a per-user workspace for system-process operations.
    ///
    /// Used by the heartbeat and routine engine to get a workspace scoped to
    /// a specific user without exposing the raw database handle.
    pub fn workspace_for_user(&self, user_id: impl Into<String>) -> Workspace {
        Workspace::new_with_db(user_id, Arc::clone(&self.inner))
    }

    // === Routine engine ===

    pub async fn list_all_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_all_routines().await
    }

    pub async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_event_routines().await
    }

    pub async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.inner.list_due_cron_routines().await
    }

    pub async fn list_dispatched_routine_runs(&self) -> Result<Vec<RoutineRun>, DatabaseError> {
        self.inner.list_dispatched_routine_runs().await
    }

    pub async fn count_running_routine_runs_batch(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, i64>, DatabaseError> {
        self.inner
            .count_running_routine_runs_batch(routine_ids)
            .await
    }

    pub async fn batch_get_last_run_status(
        &self,
        routine_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, RunStatus>, DatabaseError> {
        self.inner.batch_get_last_run_status(routine_ids).await
    }

    pub async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        self.inner.count_running_routine_runs(routine_id).await
    }

    pub async fn update_routine_runtime(
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

    pub async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        self.inner.create_routine_run(run).await
    }

    pub async fn complete_routine_run(
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

    pub async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.inner.link_routine_run_to_job(run_id, job_id).await
    }

    pub async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        self.inner.get_routine(id).await
    }

    pub async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.inner.update_routine(routine).await
    }

    // === Self-repair ===

    pub async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        self.inner.get_stuck_jobs().await
    }

    pub async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        self.inner.get_broken_tools(threshold).await
    }

    pub async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        self.inner
            .record_tool_failure(tool_name, error_message)
            .await
    }

    pub async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.inner.mark_tool_repaired(tool_name).await
    }

    pub async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.inner.increment_repair_attempts(tool_name).await
    }

    // === Sandbox housekeeping ===

    pub async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        self.inner.cleanup_stale_sandbox_jobs().await
    }

    pub async fn get_sandbox_job(
        &self,
        id: Uuid,
    ) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        self.inner.get_sandbox_job(id).await
    }

    pub async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        self.inner.save_sandbox_job(job).await
    }

    pub async fn update_sandbox_job_status(
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

    pub async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        self.inner.update_sandbox_job_mode(id, mode).await
    }

    pub async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        self.inner.get_sandbox_job_mode(id).await
    }

    pub async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.inner.save_job_event(job_id, event_type, data).await
    }

    pub async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<crate::history::JobEventRecord>, DatabaseError> {
        self.inner.list_job_events(job_id, limit).await
    }

    // === Job persistence (scheduler, worker) ===

    pub async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        self.inner.get_job(id).await
    }

    pub async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        self.inner.save_job(ctx).await
    }

    pub async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.inner
            .update_job_status(id, status, failure_reason)
            .await
    }

    pub async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.inner.mark_job_stuck(id).await
    }

    pub async fn list_agent_jobs(&self) -> Result<Vec<AgentJobRecord>, DatabaseError> {
        self.inner.list_agent_jobs().await
    }

    pub async fn get_agent_job_failure_reason(
        &self,
        id: Uuid,
    ) -> Result<Option<String>, DatabaseError> {
        self.inner.get_agent_job_failure_reason(id).await
    }

    // === LLM call recording ===

    pub async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        self.inner.record_llm_call(record).await
    }

    pub async fn save_action(
        &self,
        job_id: Uuid,
        action: &ActionRecord,
    ) -> Result<(), DatabaseError> {
        self.inner.save_action(job_id, action).await
    }

    pub async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        self.inner.get_job_actions(job_id).await
    }

    // === Estimation ===

    pub async fn save_estimation_snapshot(
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

    pub async fn update_estimation_actuals(
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

    // === Conversations (system context) ===

    pub async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .add_conversation_message(conversation_id, role, content)
            .await
    }

    pub async fn get_or_create_routine_conversation(
        &self,
        routine_id: Uuid,
        routine_name: &str,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .get_or_create_routine_conversation(routine_id, routine_name, user_id)
            .await
    }

    pub async fn get_or_create_heartbeat_conversation(
        &self,
        user_id: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.inner
            .get_or_create_heartbeat_conversation(user_id)
            .await
    }
}

// ---------------------------------------------------------------------------
// AdminScope — human admin operations, requires UserRole::Admin
// ---------------------------------------------------------------------------

/// Database access for human admin operations.
///
/// Constructable only with `UserRole::Admin`. Returns `None` if the identity
/// is not an admin. Currently exposes user management only.
#[derive(Clone)]
pub struct AdminScope {
    inner: Arc<dyn Database>,
    #[allow(dead_code)]
    identity: crate::ownership::Identity,
}

impl AdminScope {
    /// Construct an `AdminScope`. Returns `None` if the identity is not `Admin`.
    pub fn new(identity: crate::ownership::Identity, db: Arc<dyn Database>) -> Option<Self> {
        if identity.role != crate::ownership::UserRole::Admin {
            return None;
        }
        Some(Self {
            inner: db,
            identity,
        })
    }

    // === User management ===

    pub async fn list_users(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<crate::db::UserRecord>, crate::error::DatabaseError> {
        self.inner.list_users(status).await
    }

    pub async fn get_user(
        &self,
        id: &str,
    ) -> Result<Option<crate::db::UserRecord>, crate::error::DatabaseError> {
        self.inner.get_user(id).await
    }

    pub async fn create_user(
        &self,
        user: &crate::db::UserRecord,
    ) -> Result<(), crate::error::DatabaseError> {
        self.inner.create_user(user).await
    }

    pub async fn deactivate_user(&self, id: &str) -> Result<(), crate::error::DatabaseError> {
        self.inner.update_user_status(id, "deactivated").await
    }
}

// ---------------------------------------------------------------------------
// TenantRateState / TenantRateRegistry — per-user concurrency
// ---------------------------------------------------------------------------

/// Per-tenant concurrency limits.
pub struct TenantRateState {
    /// Limits concurrent LLM calls for this user.
    pub llm_semaphore: Arc<Semaphore>,
    /// Limits concurrent jobs for this user.
    pub job_semaphore: Arc<Semaphore>,
}

impl TenantRateState {
    pub fn new(max_llm_concurrent: usize, max_job_concurrent: usize) -> Self {
        Self {
            llm_semaphore: Arc::new(Semaphore::new(max_llm_concurrent)),
            job_semaphore: Arc::new(Semaphore::new(max_job_concurrent)),
        }
    }
}

/// Registry that lazily creates per-tenant rate state.
///
/// Uses `tokio::sync::RwLock<HashMap>` (consistent with the rest of the
/// codebase — no DashMap dependency).
pub struct TenantRateRegistry {
    state: tokio::sync::RwLock<HashMap<String, Arc<TenantRateState>>>,
    max_llm_concurrent: usize,
    max_job_concurrent: usize,
}

impl TenantRateRegistry {
    pub fn new(max_llm_concurrent: usize, max_job_concurrent: usize) -> Self {
        Self {
            state: tokio::sync::RwLock::new(HashMap::new()),
            max_llm_concurrent,
            max_job_concurrent,
        }
    }

    /// Get or lazily create rate state for a user.
    pub async fn get_or_create(&self, user_id: &str) -> Arc<TenantRateState> {
        // Fast path: read lock
        {
            let map = self.state.read().await;
            if let Some(s) = map.get(user_id) {
                return Arc::clone(s);
            }
        }
        // Slow path: write lock with double-check
        let mut map = self.state.write().await;
        if let Some(s) = map.get(user_id) {
            return Arc::clone(s);
        }
        let s = Arc::new(TenantRateState::new(
            self.max_llm_concurrent,
            self.max_job_concurrent,
        ));
        map.insert(user_id.to_string(), Arc::clone(&s));
        s
    }
}

// ---------------------------------------------------------------------------
// UserQuota — per-user quota from the database
// ---------------------------------------------------------------------------

/// Per-user quota loaded from the `users` table.
///
/// `None` fields mean no quota is assigned for that dimension.
/// Under fail-closed semantics, a non-admin user with no quota is denied.
#[derive(Debug, Clone)]
pub struct UserQuota {
    /// Maximum number of routines (agents) this user can create.
    pub max_routines: Option<i32>,
    /// Maximum daily LLM spend in cents (e.g. 10000 = $100/day).
    pub max_cost_per_day_cents: Option<i64>,
}

impl UserQuota {
    /// Returns `true` if at least one quota dimension is set.
    pub fn has_any_limit(&self) -> bool {
        self.max_routines.is_some() || self.max_cost_per_day_cents.is_some()
    }
}

// ---------------------------------------------------------------------------
// TenantCtx — per-request tenant execution context
// ---------------------------------------------------------------------------

/// Per-request tenant execution context.
///
/// Bundles a [`TenantScope`] (scoped DB access), workspace, cost guard,
/// per-tenant rate limiting, and per-user quota. Constructed once per
/// request via [`AgentDeps::tenant_ctx()`](crate::agent::AgentDeps::tenant_ctx).
///
/// **Quota enforcement (fail-closed):** Non-admin users without an explicit
/// quota assignment are denied LLM calls and routine creation. Admin users
/// are always exempt from quota checks.
///
/// `Clone + Send + Sync` — safe to store on `ChatDelegate` without lifetime issues.
#[derive(Clone)]
pub struct TenantCtx {
    identity: crate::ownership::Identity,
    store: Option<TenantScope>,
    workspace: Option<Arc<Workspace>>,
    cost_guard: Arc<CostGuard>,
    rate: Arc<TenantRateState>,
    /// Per-user quota from the DB. `None` = no quota assigned (fail-closed
    /// for non-admin users).
    user_quota: Option<UserQuota>,
}

impl TenantCtx {
    pub fn new(
        identity: crate::ownership::Identity,
        store: Option<TenantScope>,
        workspace: Option<Arc<Workspace>>,
        cost_guard: Arc<CostGuard>,
        rate: Arc<TenantRateState>,
    ) -> Self {
        Self {
            identity,
            store,
            workspace,
            cost_guard,
            rate,
            user_quota: None,
        }
    }

    /// Construct with an explicit user quota (loaded from DB at request time).
    pub fn with_quota(mut self, quota: Option<UserQuota>) -> Self {
        self.user_quota = quota;
        self
    }

    pub fn user_id(&self) -> &str {
        self.identity.owner_id.as_str()
    }

    pub fn identity(&self) -> &crate::ownership::Identity {
        &self.identity
    }

    pub fn store(&self) -> Option<&TenantScope> {
        self.store.as_ref()
    }

    pub fn workspace(&self) -> Option<&Arc<Workspace>> {
        self.workspace.as_ref()
    }

    pub fn cost_guard(&self) -> &CostGuard {
        &self.cost_guard
    }

    /// Per-user quota, if loaded from DB.
    pub fn user_quota(&self) -> Option<&UserQuota> {
        self.user_quota.as_ref()
    }

    /// Returns `true` if this user is an admin (exempt from quota checks).
    pub fn is_admin(&self) -> bool {
        self.identity.role == crate::ownership::UserRole::Admin
    }

    /// Check cost limits for this tenant (fail-closed quota + global + per-user).
    ///
    /// Enforcement order:
    /// 1. Admin users are always exempt.
    /// 2. Non-admin users without any quota assignment are denied.
    /// 3. Per-user DB quota is checked against daily spend.
    /// 4. Global limits (`CostGuard`) are checked as a ceiling.
    pub async fn check_cost_allowed(&self) -> Result<(), CostLimitExceeded> {
        // Admin users bypass all quota checks.
        if self.is_admin() {
            return Ok(());
        }

        // Fail-closed: non-admin without quota is denied.
        let quota = match &self.user_quota {
            Some(q) => q,
            None => {
                return Err(CostLimitExceeded::NoQuotaAssigned {
                    user_id: self.identity.owner_id.as_str().to_string(),
                });
            }
        };

        // Check per-user daily cost limit from DB.
        if let Some(limit_cents) = quota.max_cost_per_day_cents {
            let limit_cents_u64 = limit_cents.max(0) as u64;
            let spent = self
                .cost_guard
                .daily_spend_for_user(self.identity.owner_id.as_str())
                .await;
            let spent_cents = {
                let c = (spent * rust_decimal_macros::dec!(100)).trunc();
                c.to_string().parse::<u64>().unwrap_or(0)
            };
            if spent_cents >= limit_cents_u64 {
                return Err(CostLimitExceeded::UserDailyBudget {
                    user_id: self.identity.owner_id.as_str().to_string(),
                    spent_cents,
                    limit_cents: limit_cents_u64,
                });
            }
        }

        // Also check global limits (daily budget, hourly rate, global per-user).
        self.cost_guard
            .check_allowed_for_user(self.identity.owner_id.as_str())
            .await
    }

    /// Record an LLM call for this tenant.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_llm_call(
        &self,
        model: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_input_tokens: u32,
        cache_creation_input_tokens: u32,
        cache_read_discount: Decimal,
        cache_write_multiplier: Decimal,
        cost_per_token: Option<(Decimal, Decimal)>,
    ) -> Decimal {
        self.cost_guard
            .record_llm_call_for_user(
                self.identity.owner_id.as_str(),
                model,
                input_tokens,
                output_tokens,
                cache_read_input_tokens,
                cache_creation_input_tokens,
                cache_read_discount,
                cache_write_multiplier,
                cost_per_token,
            )
            .await
    }

    /// Acquire an LLM concurrency permit for this tenant.
    pub async fn acquire_llm_permit(&self) -> Result<SemaphorePermit<'_>, crate::error::Error> {
        self.rate.llm_semaphore.acquire().await.map_err(|_| {
            crate::error::Error::Config(crate::error::ConfigError::InvalidValue {
                key: "llm_semaphore".to_string(),
                message: "semaphore closed".to_string(),
            })
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ownership::{Identity, OwnerId, UserRole};

    fn alice_identity() -> Identity {
        Identity::new(OwnerId::from("alice"), UserRole::Member)
    }

    fn admin_identity() -> Identity {
        Identity::new(OwnerId::from("admin-user"), UserRole::Admin)
    }

    async fn test_db() -> Arc<dyn crate::db::Database> {
        let backend = crate::db::libsql::LibSqlBackend::new_memory()
            .await
            .unwrap();
        Arc::new(backend)
    }

    // ---- TenantScope tests ----

    #[tokio::test]
    async fn test_tenant_scope_with_identity_carries_owner_id() {
        let scope = TenantScope::with_identity(alice_identity(), test_db().await);
        assert_eq!(scope.user_id(), "alice");
        assert_eq!(scope.identity().owner_id.as_str(), "alice");
        assert_eq!(scope.identity().role, UserRole::Member);
    }

    #[tokio::test]
    async fn test_tenant_scope_new_bridge_creates_member_identity() {
        let scope = TenantScope::new("alice", test_db().await);
        assert_eq!(scope.user_id(), "alice");
        assert_eq!(scope.identity().role, UserRole::Member);
    }

    // ---- AdminScope tests ----

    #[tokio::test]
    async fn test_admin_scope_new_returns_some_for_admin() {
        let scope = AdminScope::new(admin_identity(), test_db().await);
        assert!(
            scope.is_some(),
            "Admin identity should construct AdminScope"
        );
    }

    #[tokio::test]
    async fn test_admin_scope_new_returns_none_for_member() {
        let scope = AdminScope::new(alice_identity(), test_db().await);
        assert!(
            scope.is_none(),
            "Member identity should NOT construct AdminScope"
        );
    }

    #[tokio::test]
    async fn test_tenant_scope_identity_accessible_after_with_identity() {
        let scope = TenantScope::with_identity(admin_identity(), test_db().await);
        assert_eq!(scope.identity().role, UserRole::Admin);
        assert_eq!(scope.user_id(), "admin-user");
    }

    // ---- TenantRateRegistry tests ----

    #[tokio::test]
    async fn test_rate_registry_returns_same_state_for_same_user() {
        let registry = TenantRateRegistry::new(4, 3);
        let a1 = registry.get_or_create("alice").await;
        let a2 = registry.get_or_create("alice").await;
        assert!(Arc::ptr_eq(&a1, &a2));
    }

    #[tokio::test]
    async fn test_rate_registry_different_users_get_different_state() {
        let registry = TenantRateRegistry::new(4, 3);
        let alice = registry.get_or_create("alice").await;
        let bob = registry.get_or_create("bob").await;
        assert!(!Arc::ptr_eq(&alice, &bob));
    }
}
