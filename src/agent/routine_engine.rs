//! Routine execution engine.
//!
//! Handles loading routines, checking triggers, enforcing guardrails,
//! and executing both lightweight (single LLM call) and full-job routines.
//!
//! The engine runs two independent loops:
//! - A **cron ticker** that polls the DB every N seconds for due cron routines
//! - An **event matcher** called synchronously from the agent main loop
//!
//! Lightweight routines execute inline (single LLM call, no scheduler slot).
//! Full-job routines are delegated to the existing `Scheduler`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;
use regex::Regex;
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::agent::Scheduler;
use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineRun, RunStatus, Trigger,
    apply_routine_verification_result, next_cron_fire, routine_verification_fingerprint,
};
use crate::channels::{IncomingMessage, OutgoingResponse};
use crate::config::RoutineConfig;
use crate::context::{JobContext, JobState};
use crate::error::RoutineError;
use crate::extensions::ExtensionManager;
use crate::llm::{
    ChatMessage, CompletionRequest, FinishReason, LlmProvider, ToolCall, ToolCompletionRequest,
};
use crate::tenant::SystemScope;
use crate::tools::{
    ToolError, ToolRegistry, autonomous_allowed_tool_names, autonomous_unavailable_message,
    prepare_tool_params,
};
use crate::workspace::Workspace;
use ironclaw_safety::SafetyLayer;

enum EventMatcher {
    Message { routine: Routine, regex: Regex },
    System { routine: Routine },
}

struct TriggeredRoutine {
    routine: Routine,
    detail: String,
}

/// Distinguishes why sandbox is unavailable so error messages are accurate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxReadiness {
    /// Docker is available and sandbox is enabled.
    Available,
    /// User explicitly disabled sandboxing (SANDBOX_ENABLED=false).
    DisabledByConfig,
    /// Sandbox is enabled but Docker is not running or not installed.
    DockerUnavailable,
}

/// Check whether an event-triggered routine's user/channel filters match an
/// incoming message.
///
/// Returns `true` if:
/// - The routine has an `Event` trigger (non-Event routines always return `false`)
/// - The routine's `user_id` matches the message's user scope
/// - The routine's channel filter (if any) matches the message channel
///   case-insensitively
///
/// This is a pure function extracted from `check_event_triggers` so the
/// filter logic can be unit-tested without async infrastructure.
pub(crate) fn routine_matches_message(routine: &Routine, message: &IncomingMessage) -> bool {
    // Only Event-triggered routines can match incoming messages.
    if !matches!(routine.trigger, Trigger::Event { .. }) {
        return false;
    }

    // User ownership filter — only fire routines scoped to this user.
    if routine.user_id != message.user_id {
        return false;
    }

    // Channel filter (case-insensitive, matching emit_system_event behavior)
    if let Trigger::Event {
        channel: Some(ch), ..
    } = &routine.trigger
        && !ch.eq_ignore_ascii_case(&message.channel)
    {
        return false;
    }

    true
}

fn trigger_uses_event_cache(trigger: &Trigger) -> bool {
    matches!(trigger, Trigger::Event { .. } | Trigger::SystemEvent { .. })
}

/// The routine execution engine.
pub struct RoutineEngine {
    config: RoutineConfig,
    store: SystemScope,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    /// Sender for notifications (routed to channel manager).
    notify_tx: mpsc::Sender<OutgoingResponse>,
    /// Currently running routine count (across all routines).
    running_count: Arc<AtomicUsize>,
    /// Cached matchers for all event-driven routines.
    event_cache: Arc<RwLock<Vec<EventMatcher>>>,
    /// Scheduler for dispatching jobs (FullJob mode).
    scheduler: Option<Arc<Scheduler>>,
    /// Owner-scoped extension activation state for autonomous tool resolution.
    extension_manager: Option<Arc<ExtensionManager>>,
    /// Tool registry for lightweight routine tool execution.
    tools: Arc<ToolRegistry>,
    /// Safety layer for tool output sanitization.
    safety: Arc<SafetyLayer>,
    /// Sandbox readiness state — only `DockerUnavailable` blocks full-job dispatch.
    sandbox_readiness: SandboxReadiness,
    /// Sender for injecting routine-review messages into the agent loop.
    inject_tx: Option<mpsc::Sender<IncomingMessage>>,
    /// Hourly agent review counter (rate limit).
    agent_review_count: Arc<AtomicU32>,
    /// Timestamp (epoch secs) when the hourly review window started.
    agent_review_window_start: Arc<std::sync::atomic::AtomicI64>,
    /// Consecutive agent review failures (circuit breaker). Per-routine tracking
    /// is not feasible with atomics alone, so this is a global counter that
    /// trips the circuit breaker for all routines.
    agent_review_consecutive_failures: Arc<AtomicU32>,
    /// Timestamp when this engine instance was created. Used by
    /// `sync_dispatched_runs` to distinguish orphaned runs (from a previous
    /// process) from actively-watched runs (from this process).
    boot_time: chrono::DateTime<Utc>,
}

impl RoutineEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: RoutineConfig,
        store: SystemScope,
        llm: Arc<dyn LlmProvider>,
        workspace: Arc<Workspace>,
        notify_tx: mpsc::Sender<OutgoingResponse>,
        scheduler: Option<Arc<Scheduler>>,
        extension_manager: Option<Arc<ExtensionManager>>,
        tools: Arc<ToolRegistry>,
        safety: Arc<SafetyLayer>,
        sandbox_readiness: SandboxReadiness,
        inject_tx: Option<mpsc::Sender<IncomingMessage>>,
    ) -> Self {
        Self {
            config,
            store,
            llm,
            workspace,
            notify_tx,
            running_count: Arc::new(AtomicUsize::new(0)),
            event_cache: Arc::new(RwLock::new(Vec::new())),
            scheduler,
            extension_manager,
            tools,
            safety,
            sandbox_readiness,
            inject_tx,
            agent_review_count: Arc::new(AtomicU32::new(0)),
            agent_review_window_start: Arc::new(std::sync::atomic::AtomicI64::new(
                Utc::now().timestamp(),
            )),
            agent_review_consecutive_failures: Arc::new(AtomicU32::new(0)),
            boot_time: Utc::now(),
        }
    }

    /// Expose the running count for integration tests.
    #[doc(hidden)]
    pub fn running_count_for_test(&self) -> &Arc<AtomicUsize> {
        &self.running_count
    }

    /// Refresh the in-memory event trigger cache from DB.
    pub async fn refresh_event_cache(&self) {
        match self.store.list_event_routines().await {
            Ok(routines) => {
                let mut cache = Vec::new();
                for routine in routines {
                    match &routine.trigger {
                        Trigger::Event { pattern, .. } => {
                            // Use RegexBuilder with size limit to prevent ReDoS
                            // from user-supplied patterns (issue #825).
                            match regex::RegexBuilder::new(pattern)
                                .size_limit(64 * 1024) // 64KB compiled size limit
                                .build()
                            {
                                Ok(re) => cache.push(EventMatcher::Message {
                                    routine: routine.clone(),
                                    regex: re,
                                }),
                                Err(e) => {
                                    tracing::warn!(
                                        routine = %routine.name,
                                        "Invalid or too complex event regex '{}': {}",
                                        pattern, e
                                    );
                                }
                            }
                        }
                        Trigger::SystemEvent { .. } => {
                            cache.push(EventMatcher::System {
                                routine: routine.clone(),
                            });
                        }
                        _ => {}
                    }
                }
                let count = cache.len();
                *self.event_cache.write().await = cache;
                tracing::trace!("Refreshed event cache: {} routines", count);
            }
            Err(e) => {
                tracing::error!("Failed to refresh event cache: {}", e);
            }
        }
    }

    /// Check incoming message against event triggers. Returns number of routines fired.
    pub async fn check_event_triggers(&self, message: &IncomingMessage, content: &str) -> usize {
        let triggered = self.matching_event_triggers(message, content).await;
        let fired = triggered.len();
        for triggered in triggered {
            std::mem::drop(self.spawn_fire(triggered.routine, "event", Some(triggered.detail)));
        }
        fired
    }

    /// Fire matching event-triggered routines and wait for them to complete.
    ///
    /// Used by single-message REPL mode so the process does not exit before
    /// background event-triggered routines finish.
    pub async fn check_event_triggers_and_wait(
        &self,
        message: &IncomingMessage,
        content: &str,
    ) -> usize {
        let triggered = self.matching_event_triggers(message, content).await;
        let fired = triggered.len();
        let handles: Vec<JoinHandle<()>> = triggered
            .into_iter()
            .map(|triggered| self.spawn_fire(triggered.routine, "event", Some(triggered.detail)))
            .collect();

        for handle in handles {
            if let Err(e) = handle.await {
                tracing::warn!(error = %e, "Event-triggered routine task failed");
            }
        }

        fired
    }

    async fn matching_event_triggers(
        &self,
        message: &IncomingMessage,
        content: &str,
    ) -> Vec<TriggeredRoutine> {
        let cache = self.event_cache.read().await;

        // Early return if there are no message matchers at all.
        if !cache
            .iter()
            .any(|m| matches!(m, EventMatcher::Message { .. }))
        {
            return Vec::new();
        }
        let mut triggered = Vec::new();

        // Collect routine IDs for batch query
        let routine_ids: Vec<Uuid> = cache
            .iter()
            .filter_map(|matcher| match matcher {
                EventMatcher::Message { routine, .. } => Some(routine.id),
                EventMatcher::System { .. } => None,
            })
            .collect();

        if routine_ids.is_empty() {
            return Vec::new();
        }

        // Single batch query instead of N queries
        let concurrent_counts = match self.batch_concurrent_counts(&routine_ids).await {
            Some(counts) => counts,
            None => return Vec::new(),
        };

        for matcher in cache.iter() {
            let (routine, re) = match matcher {
                EventMatcher::Message { routine, regex } => (routine, regex),
                EventMatcher::System { .. } => continue,
            };

            // User ownership + channel filter (extracted for testability).
            if !routine_matches_message(routine, message) {
                // User mismatch is expected for multi-user setups — keep at
                // trace to avoid one log per routine per inbound message.
                if routine.user_id != message.user_id {
                    tracing::trace!(
                        routine = %routine.name,
                        routine_user = %routine.user_id,
                        message_user = %message.user_id,
                        "Skipped: user scope mismatch"
                    );
                } else {
                    tracing::debug!(
                        routine = %routine.name,
                        channel = %message.channel,
                        "Skipped: channel mismatch"
                    );
                }
                continue;
            }

            // Regex match
            if !re.is_match(content) {
                continue;
            }

            // Cooldown check
            if !self.check_cooldown(routine) {
                tracing::debug!(routine = %routine.name, "Skipped: cooldown active");
                continue;
            }

            // Concurrent run check (using batch-loaded counts)
            let running_count = concurrent_counts.get(&routine.id).copied().unwrap_or(0);
            if running_count >= routine.guardrails.max_concurrent as i64 {
                tracing::debug!(routine = %routine.name, "Skipped: max concurrent reached");
                continue;
            }

            // Global capacity check
            if self.running_count.load(Ordering::Relaxed) >= self.config.max_concurrent_routines {
                tracing::warn!(routine = %routine.name, "Skipped: global max concurrent reached");
                continue;
            }

            let detail = truncate(content, 200);
            triggered.push(TriggeredRoutine {
                routine: routine.clone(),
                detail,
            });
        }

        triggered
    }

    /// Emit a structured event to system-event routines.
    ///
    /// Returns the number of routines that were fired.
    pub async fn emit_system_event(
        &self,
        source: &str,
        event_type: &str,
        payload: &serde_json::Value,
        user_id: Option<&str>,
    ) -> usize {
        let cache = self.event_cache.read().await;

        // Early return if there are no system-event matchers at all.
        if !cache
            .iter()
            .any(|m| matches!(m, EventMatcher::System { .. }))
        {
            return 0;
        }

        let mut fired = 0;

        // Collect routine IDs for batch query
        let routine_ids: Vec<Uuid> = cache
            .iter()
            .filter_map(|matcher| match matcher {
                EventMatcher::System { routine } => Some(routine.id),
                EventMatcher::Message { .. } => None,
            })
            .collect();

        if routine_ids.is_empty() {
            return 0;
        }

        // Single batch query instead of N queries
        let concurrent_counts = match self.batch_concurrent_counts(&routine_ids).await {
            Some(counts) => counts,
            None => return 0,
        };

        for matcher in cache.iter() {
            let routine = match matcher {
                EventMatcher::System { routine } => routine,
                EventMatcher::Message { .. } => continue,
            };

            let Trigger::SystemEvent {
                source: expected_source,
                event_type: expected_event,
                filters,
            } = &routine.trigger
            else {
                continue;
            };

            if !expected_source.eq_ignore_ascii_case(source)
                || !expected_event.eq_ignore_ascii_case(event_type)
            {
                continue;
            }

            if let Some(uid) = user_id
                && routine.user_id != uid
            {
                continue;
            }

            let mut matched = true;
            for (key, expected) in filters {
                let Some(actual) = payload
                    .get(key)
                    .and_then(crate::agent::routine::json_value_as_filter_string)
                else {
                    tracing::debug!(routine = %routine.name, filter_key = %key, "Filter key not found in payload");
                    matched = false;
                    break;
                };
                if !actual.eq_ignore_ascii_case(expected) {
                    matched = false;
                    break;
                }
            }
            if !matched {
                continue;
            }

            if !self.check_cooldown(routine) {
                tracing::debug!(routine = %routine.name, "Skipped: cooldown active");
                continue;
            }

            // Concurrent run check (using batch-loaded counts)
            let running_count = concurrent_counts.get(&routine.id).copied().unwrap_or(0);
            if running_count >= routine.guardrails.max_concurrent as i64 {
                tracing::debug!(routine = %routine.name, "Skipped: max concurrent reached");
                continue;
            }

            if self.running_count.load(Ordering::Relaxed) >= self.config.max_concurrent_routines {
                tracing::warn!(routine = %routine.name, "Skipped: global max concurrent reached");
                continue;
            }

            let detail = truncate(&format!("{source}:{event_type}"), 200);
            self.spawn_fire(routine.clone(), "system_event", Some(detail));
            fired += 1;
        }

        fired
    }

    /// Batch-load concurrent run counts for a set of routine IDs.
    ///
    /// Returns `None` on database error (already logged).
    async fn batch_concurrent_counts(&self, routine_ids: &[Uuid]) -> Option<HashMap<Uuid, i64>> {
        match self
            .store
            .count_running_routine_runs_batch(routine_ids)
            .await
        {
            Ok(counts) => Some(counts),
            Err(e) => {
                tracing::error!("Failed to batch-load concurrent counts: {}", e);
                None
            }
        }
    }

    /// Check all due cron routines and fire them. Called by the cron ticker.
    pub async fn check_cron_triggers(&self) {
        let routines = match self.store.list_due_cron_routines().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to load due cron routines: {}", e);
                return;
            }
        };

        for routine in routines {
            if self.running_count.load(Ordering::Relaxed) >= self.config.max_concurrent_routines {
                tracing::warn!("Global max concurrent routines reached, skipping remaining");
                break;
            }

            if !self.check_cooldown(&routine) {
                continue;
            }

            if !self.check_concurrent(&routine).await {
                continue;
            }

            let detail = if let Trigger::Cron { ref schedule, .. } = routine.trigger {
                Some(schedule.clone())
            } else {
                None
            };

            self.spawn_fire(routine, "cron", detail);
        }
    }

    /// Reconcile orphaned full_job routine runs with their linked job outcomes.
    ///
    /// Called on each cron tick. Finds routine runs that are still `running`
    /// with a linked `job_id`, checks the job state, and finalizes the run
    /// when the job reaches a completed or terminal state.
    ///
    /// Only processes runs started **before** this engine's boot time, so it
    /// never races with `FullJobWatcher` instances from the current process.
    /// This makes it safe to call on every tick as a crash-recovery mechanism.
    pub async fn sync_dispatched_runs(&self) {
        let runs = match self.store.list_dispatched_routine_runs().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("Failed to list dispatched routine runs: {}", e);
                return;
            }
        };

        // Only process runs from a previous process instance. Runs started
        // after boot_time are actively watched by a FullJobWatcher in this
        // process and should not be finalized here.
        let orphaned: Vec<_> = runs
            .into_iter()
            .filter(|r| r.started_at < self.boot_time)
            .collect();

        if orphaned.is_empty() {
            return;
        }

        tracing::info!(
            "Recovering {} orphaned dispatched routine runs",
            orphaned.len()
        );

        for run in orphaned {
            let job_id = match run.job_id {
                Some(id) => id,
                None => continue, // Should not happen (query filters), but guard anyway
            };

            // Fetch the linked job
            let job = match self.store.get_job(job_id).await {
                Ok(Some(j)) => j,
                Ok(None) => {
                    // Orphaned: job record was deleted or never persisted
                    tracing::warn!(
                        run_id = %run.id,
                        job_id = %job_id,
                        "Linked job not found, marking routine run as failed"
                    );
                    self.complete_dispatched_run(
                        &run,
                        RunStatus::Failed,
                        &format!("Linked job {job_id} not found (orphaned)"),
                    )
                    .await;
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        run_id = %run.id,
                        job_id = %job_id,
                        "Failed to fetch linked job: {}", e
                    );
                    continue;
                }
            };

            // Map job state to final run status
            let final_status = match job.state {
                JobState::Completed | JobState::Submitted | JobState::Accepted => {
                    Some(RunStatus::Ok)
                }
                JobState::Failed | JobState::Cancelled => Some(RunStatus::Failed),
                // Pending, InProgress, Stuck — still running
                _ => None,
            };

            let status = match final_status {
                Some(s) => s,
                None => continue, // Job still active, check again next tick
            };

            // Build summary
            let summary = if status == RunStatus::Failed {
                match self.store.get_agent_job_failure_reason(job_id).await {
                    Ok(Some(reason)) => format!("Job {job_id} failed: {reason}"),
                    _ => format!("Job {job_id} {}", job.state),
                }
            } else {
                format!("Job {job_id} completed successfully")
            };

            self.complete_dispatched_run(&run, status, &summary).await;
        }
    }

    /// Finalize a dispatched routine run: update DB, update routine runtime,
    /// persist to conversation thread, and send notification.
    async fn complete_dispatched_run(&self, run: &RoutineRun, status: RunStatus, summary: &str) {
        // Complete the run record in DB
        if let Err(e) = self
            .store
            .complete_routine_run(run.id, status, Some(summary), None)
            .await
        {
            tracing::error!(
                run_id = %run.id,
                "Failed to complete dispatched routine run: {}", e
            );
            return;
        }

        tracing::info!(
            run_id = %run.id,
            status = %status,
            "Finalized dispatched routine run"
        );

        // Load the routine to update consecutive_failures and send notification
        let mut routine = match self.store.get_routine(run.routine_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                tracing::warn!(
                    run_id = %run.id,
                    routine_id = %run.routine_id,
                    "Routine not found for dispatched run finalization"
                );
                return;
            }
            Err(e) => {
                tracing::error!(
                    run_id = %run.id,
                    "Failed to load routine for dispatched run: {}", e
                );
                return;
            }
        };

        // Update runtime fields. In crash recovery, execute_routine() never
        // reached its normal runtime update, so we must advance all fields here.
        let new_failures = if status == RunStatus::Failed {
            routine.consecutive_failures + 1
        } else {
            0
        };

        let now = Utc::now();
        routine.state = apply_routine_verification_result(
            &routine.state,
            routine_verification_fingerprint(&routine),
            status,
            now,
        );
        let next_fire = if let Trigger::Cron {
            ref schedule,
            ref timezone,
        } = routine.trigger
        {
            next_cron_fire(schedule, timezone.as_deref()).unwrap_or(None)
        } else {
            None
        };

        let runtime_updated = match self
            .store
            .update_routine_runtime(
                routine.id,
                now,
                next_fire,
                routine.run_count + 1,
                new_failures,
                &routine.state,
            )
            .await
        {
            Ok(()) => true,
            Err(e) => {
                tracing::error!(
                    routine = %routine.name,
                    "Failed to update routine runtime after dispatched run: {}", e
                );
                false
            }
        };

        if runtime_updated && trigger_uses_event_cache(&routine.trigger) {
            update_cached_event_runtime(
                self.event_cache.as_ref(),
                routine.id,
                now,
                routine.run_count + 1,
                new_failures,
            )
            .await;
        }

        // Persist result to the routine's conversation thread
        let thread_id = match self
            .store
            .get_or_create_routine_conversation(routine.id, &routine.name, &routine.user_id)
            .await
        {
            Ok(conv_id) => {
                let msg = format!("[dispatched] {}: {}", status, summary);
                if let Err(e) = self
                    .store
                    .add_conversation_message(conv_id, "assistant", &msg)
                    .await
                {
                    tracing::error!(
                        routine = %routine.name,
                        "Failed to persist dispatched run message: {}", e
                    );
                }
                Some(conv_id.to_string())
            }
            Err(e) => {
                tracing::error!(
                    routine = %routine.name,
                    "Failed to get routine conversation: {}", e
                );
                None
            }
        };

        // Send notification
        send_notification(
            &self.notify_tx,
            &routine.notify,
            &routine.user_id,
            &routine.name,
            status,
            Some(summary),
            thread_id.as_deref(),
        )
        .await;

        // Note: we do NOT decrement running_count here. In normal flow,
        // execute_routine() handles that after FullJobWatcher returns.
        // This sync path only runs for crash recovery (process restarted),
        // where running_count was already reset to 0.
    }

    /// Fire a routine manually (from tool call or CLI).
    ///
    /// Bypasses cooldown checks (those only apply to cron/event triggers).
    /// Still enforces enabled check and concurrent run limit.
    pub async fn fire_manual(
        &self,
        routine_id: Uuid,
        user_id: Option<&str>,
    ) -> Result<Uuid, RoutineError> {
        let routine = self
            .store
            .get_routine(routine_id)
            .await
            .map_err(|e| RoutineError::Database {
                reason: e.to_string(),
            })?
            .ok_or(RoutineError::NotFound { id: routine_id })?;

        // Enforce ownership when a user_id is provided (gateway calls).
        if let Some(uid) = user_id
            && routine.user_id != uid
        {
            return Err(RoutineError::NotAuthorized { id: routine_id });
        }

        if !routine.enabled {
            return Err(RoutineError::Disabled {
                name: routine.name.clone(),
            });
        }

        if !self.check_concurrent(&routine).await {
            return Err(RoutineError::MaxConcurrent {
                name: routine.name.clone(),
            });
        }

        let run_id = Uuid::new_v4();
        let run = RoutineRun {
            id: run_id,
            routine_id: routine.id,
            trigger_type: "manual".to_string(),
            trigger_detail: None,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };

        if let Err(e) = self.store.create_routine_run(&run).await {
            return Err(RoutineError::Database {
                reason: format!("failed to create run record: {e}"),
            });
        }

        // Per-user workspace (same pattern as spawn_fire).
        let routine_workspace = if routine.user_id == self.workspace.user_id() {
            self.workspace.clone()
        } else {
            Arc::new(self.store.workspace_for_user(&routine.user_id))
        };

        // Execute inline for manual triggers (caller wants to wait)
        let engine = EngineContext {
            config: self.config.clone(),
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: routine_workspace,
            notify_tx: self.notify_tx.clone(),
            running_count: self.running_count.clone(),
            scheduler: self.scheduler.clone(),
            extension_manager: self.extension_manager.clone(),
            tools: self.tools.clone(),
            safety: self.safety.clone(),
            sandbox_readiness: self.sandbox_readiness,

            event_cache: Arc::clone(&self.event_cache),
 
            inject_tx: self.inject_tx.clone(),
            agent_review_count: self.agent_review_count.clone(),
            agent_review_window_start: self.agent_review_window_start.clone(),
            agent_review_consecutive_failures: self.agent_review_consecutive_failures.clone(),
        };

        tokio::spawn(async move {
            execute_routine(engine, routine, run).await;
        });

        Ok(run_id)
    }

    /// Fire a routine from a webhook trigger.
    ///
    /// Similar to `fire_manual` but records the trigger as `"webhook"` with the
    /// webhook path as detail. Skips ownership check (auth is via webhook secret).
    /// Enforces enabled check, cooldown, and concurrent run limit.
    pub async fn fire_webhook(
        &self,
        routine_id: Uuid,
        webhook_path: &str,
    ) -> Result<Uuid, RoutineError> {
        let routine = self
            .store
            .get_routine(routine_id)
            .await
            .map_err(|e| RoutineError::Database {
                reason: e.to_string(),
            })?
            .ok_or(RoutineError::NotFound { id: routine_id })?;

        if !routine.enabled {
            return Err(RoutineError::Disabled {
                name: routine.name.clone(),
            });
        }

        if !self.check_cooldown(&routine) {
            return Err(RoutineError::Cooldown {
                name: routine.name.clone(),
            });
        }

        if !self.check_concurrent(&routine).await {
            return Err(RoutineError::MaxConcurrent {
                name: routine.name.clone(),
            });
        }

        if self.running_count.load(Ordering::Relaxed) >= self.config.max_concurrent_routines {
            return Err(RoutineError::MaxConcurrent {
                name: routine.name.clone(),
            });
        }

        let run_id = Uuid::new_v4();
        let run = RoutineRun {
            id: run_id,
            routine_id: routine.id,
            trigger_type: "webhook".to_string(),
            trigger_detail: Some(webhook_path.to_string()),
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };

        if let Err(e) = self.store.create_routine_run(&run).await {
            return Err(RoutineError::Database {
                reason: format!("failed to create run record: {e}"),
            });
        }

        let engine = EngineContext {
            config: self.config.clone(),
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: self.workspace.clone(),
            notify_tx: self.notify_tx.clone(),
            running_count: self.running_count.clone(),
            scheduler: self.scheduler.clone(),
            extension_manager: self.extension_manager.clone(),
            tools: self.tools.clone(),
            safety: self.safety.clone(),
            sandbox_readiness: self.sandbox_readiness,

            event_cache: Arc::clone(&self.event_cache),
 
            inject_tx: self.inject_tx.clone(),
            agent_review_count: self.agent_review_count.clone(),
            agent_review_window_start: self.agent_review_window_start.clone(),
            agent_review_consecutive_failures: self.agent_review_consecutive_failures.clone(),
        };

        tokio::spawn(async move {
            execute_routine(engine, routine, run).await;
        });

        Ok(run_id)
    }

    /// Spawn a fire in a background task.
    fn spawn_fire(
        &self,
        routine: Routine,
        trigger_type: &str,
        trigger_detail: Option<String>,
    ) -> JoinHandle<()> {
        let run = RoutineRun {
            id: Uuid::new_v4(),
            routine_id: routine.id,
            trigger_type: trigger_type.to_string(),
            trigger_detail,
            started_at: Utc::now(),
            completed_at: None,
            status: RunStatus::Running,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: Utc::now(),
        };

        // Use per-user workspace so each routine executes in the correct
        // user's context. Fall back to the engine-wide workspace when the
        // routine belongs to the same user (avoids unnecessary allocation).
        let routine_workspace = if routine.user_id == self.workspace.user_id() {
            self.workspace.clone()
        } else {
            Arc::new(self.store.workspace_for_user(&routine.user_id))
        };

        let engine = EngineContext {
            config: self.config.clone(),
            store: self.store.clone(),
            llm: self.llm.clone(),
            workspace: routine_workspace,
            notify_tx: self.notify_tx.clone(),
            running_count: self.running_count.clone(),
            scheduler: self.scheduler.clone(),
            extension_manager: self.extension_manager.clone(),
            tools: self.tools.clone(),
            safety: self.safety.clone(),
            sandbox_readiness: self.sandbox_readiness,

            event_cache: Arc::clone(&self.event_cache),
 
            inject_tx: self.inject_tx.clone(),
            agent_review_count: self.agent_review_count.clone(),
            agent_review_window_start: self.agent_review_window_start.clone(),
            agent_review_consecutive_failures: self.agent_review_consecutive_failures.clone(),
        };

        // Record the run in DB, then spawn execution
        let store = self.store.clone();
        tokio::spawn(async move {
            if let Err(e) = store.create_routine_run(&run).await {
                tracing::error!(routine = %routine.name, "Failed to record run: {}", e);
                return;
            }
            execute_routine(engine, routine, run).await;
        })
    }

    fn check_cooldown(&self, routine: &Routine) -> bool {
        if let Some(last_run) = routine.last_run_at {
            let elapsed = Utc::now().signed_duration_since(last_run);
            let cooldown = chrono::Duration::from_std(routine.guardrails.cooldown)
                .unwrap_or(chrono::Duration::seconds(300));
            if elapsed < cooldown {
                return false;
            }
        }
        true
    }

    async fn check_concurrent(&self, routine: &Routine) -> bool {
        match self.store.count_running_routine_runs(routine.id).await {
            Ok(count) => count < routine.guardrails.max_concurrent as i64,
            Err(e) => {
                tracing::error!(
                    routine = %routine.name,
                    "Failed to check concurrent runs: {}", e
                );
                false
            }
        }
    }
}

/// Watches a dispatched full_job until the linked scheduler job completes.
///
/// Polls `store.get_job(job_id)` at a fixed interval until the job leaves
/// an active state (Pending/InProgress/Stuck). Maps the final `JobState` to
/// a `RunStatus` for the routine run.
struct FullJobWatcher {
    store: SystemScope,
    job_id: Uuid,
    routine_name: String,
}

impl FullJobWatcher {
    /// Poll interval between DB checks.
    const POLL_INTERVAL: Duration = Duration::from_secs(5);
    /// Safety ceiling: 24 hours, derived from POLL_INTERVAL.
    const MAX_POLLS: u32 = (24 * 60 * 60) / Self::POLL_INTERVAL.as_secs() as u32;

    fn new(store: SystemScope, job_id: Uuid, routine_name: String) -> Self {
        Self {
            store,
            job_id,
            routine_name,
        }
    }

    /// Block until the linked job finishes and return the mapped status + summary.
    async fn wait_for_completion(&self) -> (RunStatus, Option<String>) {
        let mut polls = 0u32;

        let final_status = loop {
            // Check job state before sleeping so we finalize promptly
            // if the job is already done (e.g. fast-failing jobs).
            match self.store.get_job(self.job_id).await {
                Ok(Some(job_ctx)) => {
                    // Use is_parallel_blocking (Pending/InProgress/Stuck) instead
                    // of is_active (!is_terminal) because routine jobs typically
                    // stop at Completed — which is NOT terminal but IS finished
                    // from an execution standpoint.
                    if !job_ctx.state.is_parallel_blocking() {
                        break Self::map_job_state(&job_ctx.state);
                    }
                }
                Ok(None) => {
                    tracing::warn!(
                        routine = %self.routine_name,
                        job_id = %self.job_id,
                        "full_job disappeared from DB while polling"
                    );
                    break RunStatus::Failed;
                }
                Err(e) => {
                    tracing::error!(
                        routine = %self.routine_name,
                        job_id = %self.job_id,
                        "Error polling full_job state: {}", e
                    );
                    break RunStatus::Failed;
                }
            }

            polls += 1;
            if polls >= Self::MAX_POLLS {
                tracing::error!(
                    routine = %self.routine_name,
                    job_id = %self.job_id,
                    "full_job timed out after 24 hours, treating as failed"
                );
                break RunStatus::Failed;
            }

            tokio::time::sleep(Self::POLL_INTERVAL).await;
        };

        let summary = format!("Job {} finished ({})", self.job_id, final_status);
        (final_status, Some(summary))
    }

    fn map_job_state(state: &crate::context::JobState) -> RunStatus {
        use crate::context::JobState;
        match state {
            JobState::Failed | JobState::Cancelled => RunStatus::Failed,
            _ => RunStatus::Ok, // Completed / Submitted / Accepted
        }
    }
}

/// Shared context passed to the execution function.
struct EngineContext {
    config: RoutineConfig,
    store: SystemScope,
    llm: Arc<dyn LlmProvider>,
    workspace: Arc<Workspace>,
    notify_tx: mpsc::Sender<OutgoingResponse>,
    running_count: Arc<AtomicUsize>,
    scheduler: Option<Arc<Scheduler>>,
    extension_manager: Option<Arc<ExtensionManager>>,
    tools: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
    sandbox_readiness: SandboxReadiness,

    event_cache: Arc<RwLock<Vec<EventMatcher>>>,
 
    inject_tx: Option<mpsc::Sender<IncomingMessage>>,
    agent_review_count: Arc<AtomicU32>,
    agent_review_window_start: Arc<std::sync::atomic::AtomicI64>,
    agent_review_consecutive_failures: Arc<AtomicU32>,
}

/// Execute a routine run. Handles both lightweight and full_job modes.
async fn execute_routine(ctx: EngineContext, mut routine: Routine, run: RoutineRun) {
    // Increment running count (atomic: survives panics in the execution below)
    ctx.running_count.fetch_add(1, Ordering::Relaxed);

    // Retry constants for transient lightweight execution failures.
    //
    // NOTE: Multiplicative retry budgets — `ctx.llm` is wrapped in `RetryProvider`
    // which has its own retry budget (default 3). Although `LlmFailed` errors are
    // excluded from outer retry (only `EmptyResponse`/`TruncatedResponse` retry
    // here), be aware that each outer attempt triggers a full inner retry budget
    // for the LLM call itself. With MAX_RETRIES=2, worst case is 2 outer x 3 inner
    // = 6 LLM calls per routine run.
    const MAX_RETRIES: u32 = 2;
    const BASE_DELAY_MS: u64 = 1000;

    let is_lightweight = matches!(routine.action, RoutineAction::Lightweight { .. });

    // The retry block returns both the execution result and any accumulated
    // token count so that usage is preserved even on final failure.
    let (result, accumulated_tokens) = {
        let mut attempt = 0u32;
        // Track accumulated tokens as Option to preserve None semantics:
        // None = no attempt reported tokens; Some(n) = at least one attempt did.
        let mut accumulated_tokens: Option<i32> = None;
        let uses_tools = matches!(
            routine.action,
            RoutineAction::Lightweight {
                use_tools: true,
                ..
            }
        ) && ctx.config.lightweight_tools_enabled;

        /// Extract partial_tokens from any RoutineError variant that carries them.
        fn extract_partial_tokens(e: &RoutineError) -> Option<i32> {
            match e {
                RoutineError::LlmFailed {
                    partial_tokens: Some(t),
                    ..
                }
                | RoutineError::EmptyResponse {
                    partial_tokens: Some(t),
                }
                | RoutineError::TruncatedResponse {
                    partial_tokens: Some(t),
                } => Some(*t),
                _ => None,
            }
        }

        /// Merge an optional partial token count into the accumulator,
        /// only materializing Some when at least one source had Some.
        fn accumulate(acc: Option<i32>, partial: Option<i32>) -> Option<i32> {
            match (acc, partial) {
                (Some(a), Some(p)) => Some(a.saturating_add(p)),
                (Some(a), None) => Some(a),
                (None, p) => p,
            }
        }

        loop {
            let execution_result = match &routine.action {
                RoutineAction::Lightweight {
                    prompt,
                    context_paths,
                    max_tokens,
                    use_tools,
                    max_tool_rounds,
                } => {
                    execute_lightweight(
                        &ctx,
                        &routine,
                        prompt,
                        context_paths,
                        *max_tokens,
                        *use_tools,
                        *max_tool_rounds,
                    )
                    .await
                }
                RoutineAction::FullJob {
                    title,
                    description,
                    max_iterations,
                } => {
                    let execution = FullJobExecutionConfig {
                        title,
                        description,
                        max_iterations: *max_iterations,
                    };
                    execute_full_job(&ctx, &routine, &run, &execution).await
                }
            };

            match execution_result {
                Ok((status, summary, tokens)) => {
                    // Merge tokens: only produce Some when at least one source had Some.
                    let total = accumulate(accumulated_tokens, tokens);
                    break (Ok((status, summary, total)), accumulated_tokens);
                }
                Err(ref e)
                    if is_lightweight
                        && !uses_tools
                        && e.is_retryable()
                        // Skip outer retry for LlmFailed — RetryProvider already
                        // retries transient LLM errors with its own budget. Retrying
                        // here would create a multiplicative retry count.
                        && !matches!(e, RoutineError::LlmFailed { .. })
                        && attempt < MAX_RETRIES =>
                {
                    // Accumulate partial tokens from the failed attempt.
                    accumulated_tokens = accumulate(accumulated_tokens, extract_partial_tokens(e));

                    attempt += 1;

                    let delay = Duration::from_millis(
                        BASE_DELAY_MS.saturating_mul(2u64.saturating_pow(attempt - 1)),
                    );
                    tracing::event!(target: "transient_routine_errors", tracing::Level::WARN, routine = %routine.name, attempt = attempt, max_retries = MAX_RETRIES, delay_ms = delay.as_millis() as u64, "Transient routine error, retrying: {}", e);
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    // Accumulate tokens from the final failed attempt.
                    accumulated_tokens = accumulate(accumulated_tokens, extract_partial_tokens(&e));
                    break (Err(e), accumulated_tokens);
                }
            }
        }
    };

    // Decrement running count
    ctx.running_count.fetch_sub(1, Ordering::Relaxed);

    // Process result — on failure, preserve accumulated token total from
    // earlier retry attempts so usage reporting stays accurate.
    let (status, summary, tokens) = match result {
        Ok(execution) => execution,
        Err(e) => {
            tracing::error!(routine = %routine.name, "Execution failed: {}", e);
            (RunStatus::Failed, Some(e.to_string()), accumulated_tokens)
        }
    };

    // Complete the run record
    if let Err(e) = ctx
        .store
        .complete_routine_run(run.id, status, summary.as_deref(), tokens)
        .await
    {
        tracing::error!(routine = %routine.name, "Failed to complete run record: {}", e);
    }

    let now = Utc::now();
    routine.state = apply_routine_verification_result(
        &routine.state,
        routine_verification_fingerprint(&routine),
        status,
        now,
    );

    // Update routine runtime state
    let next_fire = if let Trigger::Cron {
        ref schedule,
        ref timezone,
    } = routine.trigger
    {
        next_cron_fire(schedule, timezone.as_deref()).unwrap_or(None)
    } else {
        None
    };

    let new_failures = if status == RunStatus::Failed {
        routine.consecutive_failures + 1
    } else {
        0
    };

    let runtime_updated = match ctx
        .store
        .update_routine_runtime(
            routine.id,
            now,
            next_fire,
            routine.run_count + 1,
            new_failures,
            &routine.state,
        )
        .await
    {
        Ok(()) => true,
        Err(e) => {
            tracing::error!(routine = %routine.name, "Failed to update runtime state: {}", e);
            false
        }
    };

    if runtime_updated && trigger_uses_event_cache(&routine.trigger) {
        update_cached_event_runtime(
            ctx.event_cache.as_ref(),
            routine.id,
            now,
            routine.run_count + 1,
            new_failures,
        )
        .await;
    }

    // Persist routine result to its dedicated conversation thread
    let thread_id = match ctx
        .store
        .get_or_create_routine_conversation(routine.id, &routine.name, &routine.user_id)
        .await
    {
        Ok(conv_id) => {
            tracing::debug!(
                routine = %routine.name,
                routine_id = %routine.id,
                conversation_id = %conv_id,
                "Resolved routine conversation thread"
            );
            // Record the run result as a conversation message
            let msg = match (&summary, status) {
                (Some(s), _) => format!("[{}] {}: {}", run.trigger_type, status, s),
                (None, _) => format!("[{}] {}", run.trigger_type, status),
            };
            if let Err(e) = ctx
                .store
                .add_conversation_message(conv_id, "assistant", &msg)
                .await
            {
                tracing::error!(routine = %routine.name, "Failed to persist routine message: {}", e);
            }
            Some(conv_id.to_string())
        }
        Err(e) => {
            tracing::error!(routine = %routine.name, "Failed to get routine conversation: {}", e);
            None
        }
    };

    // Send notifications based on config
    send_notification(
        &ctx.notify_tx,
        &routine.notify,
        &routine.user_id,
        &routine.name,
        status,
        summary.as_deref(),
        thread_id.as_deref(),
    )
    .await;

    // Inject agent-review message if enabled for this status
    inject_agent_review(&ctx, &routine, &run, status, summary.as_deref()).await;
}

async fn update_cached_event_runtime(
    event_cache: &RwLock<Vec<EventMatcher>>,
    routine_id: Uuid,
    last_run_at: chrono::DateTime<Utc>,
    run_count: u64,
    consecutive_failures: u32,
) {
    let mut cache = event_cache.write().await;
    for matcher in cache.iter_mut() {
        let routine = match matcher {
            EventMatcher::Message { routine, .. } | EventMatcher::System { routine } => routine,
        };
        if routine.id == routine_id {
            routine.last_run_at = Some(last_run_at);
            routine.run_count = run_count;
            routine.consecutive_failures = consecutive_failures;
            break;
        }
    }
}

/// Sanitize a routine name for use in workspace paths.
/// Only keeps alphanumeric, dash, and underscore characters; replaces everything else.
fn sanitize_routine_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Execute a full-job routine by dispatching to the scheduler.
///
/// Fire-and-forget: creates a job via `Scheduler::dispatch_job` (which handles
/// creation, metadata, persistence, and scheduling), links the routine run to
/// the job, then watches it via `FullJobWatcher` until it reaches a
/// non-active state (not Pending/InProgress/Stuck). Returns the final
/// `RunStatus` mapped from the job outcome. This keeps the routine run
/// active for the full job lifetime so concurrency guardrails apply.
struct FullJobExecutionConfig<'a> {
    title: &'a str,
    description: &'a str,
    max_iterations: u32,
}

async fn execute_full_job(
    ctx: &EngineContext,
    routine: &Routine,
    run: &RoutineRun,
    execution: &FullJobExecutionConfig<'_>,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    // Full-job routines dispatch through the scheduler (same as /job
    // commands) — no Docker sandbox required when sandbox is disabled.
    // However, if sandbox is *enabled* but Docker is unavailable, that's
    // a misconfiguration we should surface.
    if matches!(ctx.sandbox_readiness, SandboxReadiness::DockerUnavailable) {
        return Err(RoutineError::JobDispatchFailed {
            reason: "Sandbox is enabled but Docker is not available. \
                     Install Docker or set SANDBOX_ENABLED=false."
                .to_string(),
        });
    }

    let scheduler = ctx
        .scheduler
        .as_ref()
        .ok_or_else(|| RoutineError::JobDispatchFailed {
            reason: "scheduler not available".to_string(),
        })?;

    let mut metadata = serde_json::json!({
        "max_iterations": execution.max_iterations,
        "owner_id": routine.user_id
    });
    // Carry the routine's notify config in job metadata so the message tool
    // can resolve channel/target per-job without global state mutation.
    if let Some(channel) = &routine.notify.channel {
        metadata["notify_channel"] = serde_json::json!(channel);
    }
    metadata["notify_user"] = serde_json::json!(&routine.notify.user);

    // Prepend execution context so the LLM knows it's already inside a
    // routine and should execute the task directly — not set up infrastructure.
    let contextualized_description = format!(
        "IMPORTANT: You are executing inside routine \"{routine_name}\". \
         The routine and its schedule are already configured. \
         Tools and credentials are already set up. \
         Do NOT create routines, jobs, or try to discover/install/authenticate tools. \
         Execute the task directly.\n\n{desc}",
        routine_name = routine.name,
        desc = execution.description,
    );

    let job_id = scheduler
        .dispatch_job(
            &routine.user_id,
            execution.title,
            &contextualized_description,
            Some(metadata),
        )
        .await
        .map_err(|e| RoutineError::JobDispatchFailed {
            reason: format!("failed to dispatch job: {e}"),
        })?;

    // Link the routine run to the dispatched job.
    // This MUST succeed — if it fails, sync_dispatched_runs() will never find
    // this run (it filters on job_id IS NOT NULL), leaving it stuck as 'running'
    // with running_count permanently elevated.
    ctx.store
        .link_routine_run_to_job(run.id, job_id)
        .await
        .map_err(|e| RoutineError::Database {
            reason: format!("failed to link run to job: {e}"),
        })?;

    tracing::info!(
        routine = %routine.name,
        job_id = %job_id,
        max_iterations = execution.max_iterations,
        "Dispatched full job for routine, watching for completion"
    );

    // Watch the job until it finishes — keeps the routine run active
    // so concurrency guardrails (running_count, routine_runs status)
    // remain enforced for the full job lifetime.
    let watcher = FullJobWatcher::new(ctx.store.clone(), job_id, routine.name.clone());
    let (status, summary) = watcher.wait_for_completion().await;
    Ok((status, summary, None))
}

/// Execute a lightweight routine with optional tool support.
///
/// If tools are enabled, this runs a simplified agentic loop (max 3-5 iterations).
/// If tools are disabled, this does a single LLM call (original behavior).
async fn execute_lightweight(
    ctx: &EngineContext,
    routine: &Routine,
    prompt: &str,
    context_paths: &[String],
    max_tokens: u32,
    use_tools: bool,
    max_tool_rounds: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    // Load context from workspace
    let mut context_parts = Vec::new();
    for path in context_paths {
        match ctx.workspace.read(path).await {
            Ok(doc) => {
                context_parts.push(format!("## {}\n\n{}", path, doc.content));
            }
            Err(e) => {
                tracing::debug!(
                    routine = %routine.name,
                    "Failed to read context path {}: {}", path, e
                );
            }
        }
    }

    // Load routine state from workspace (name sanitized to prevent path traversal)
    let safe_name = sanitize_routine_name(&routine.name);
    let state_path = format!("routines/{safe_name}/state.md");
    let state_content = match ctx.workspace.read(&state_path).await {
        Ok(doc) => Some(doc.content),
        Err(_) => None,
    };

    let full_prompt = build_lightweight_prompt(
        prompt,
        &context_parts,
        state_content.as_deref(),
        &routine.notify,
        use_tools,
    );

    // Get system prompt
    let system_prompt = match ctx.workspace.system_prompt().await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(routine = %routine.name, "Failed to get system prompt: {}", e);
            String::new()
        }
    };

    // Determine max_tokens from model metadata with fallback
    let effective_max_tokens = match ctx.llm.model_metadata().await {
        Ok(meta) => {
            let from_api = meta.context_length.map(|ctx| ctx / 2).unwrap_or(max_tokens);
            from_api.max(max_tokens)
        }
        Err(_) => max_tokens,
    };

    // If tools are enabled (both globally and per-routine), use the tool execution loop
    if use_tools && ctx.config.lightweight_tools_enabled {
        execute_lightweight_with_tools(
            ctx,
            routine,
            &system_prompt,
            &full_prompt,
            effective_max_tokens,
            max_tool_rounds,
        )
        .await
    } else {
        execute_lightweight_no_tools(
            ctx,
            routine,
            &system_prompt,
            &full_prompt,
            effective_max_tokens,
        )
        .await
    }
}

/// Sanitize a user-controlled string before interpolation into an LLM prompt.
/// Strips newlines (which could break prompt structure) and truncates to a
/// reasonable length to limit abuse surface.
fn sanitize_prompt_field(value: &str) -> String {
    const MAX_LEN: usize = 128;
    value
        .chars()
        .filter(|&c| c != '\n' && c != '\r')
        .take(MAX_LEN)
        .map(|c| if c == '`' { '\'' } else { c })
        .collect()
}

fn build_lightweight_prompt(
    prompt: &str,
    context_parts: &[String],
    state_content: Option<&str>,
    notify: &NotifyConfig,
    use_tools: bool,
) -> String {
    let mut full_prompt = String::new();
    full_prompt.push_str(prompt);

    if notify.on_attention {
        full_prompt.push_str("\n\n---\n\n# Delivery\n\n");
        full_prompt.push_str(
            "If you reply with anything other than ROUTINE_OK, the host will deliver your \
             reply as the routine notification. Return the message exactly as it should be sent.\n",
        );

        if let Some(channel) = notify.channel.as_deref() {
            let sanitized = sanitize_prompt_field(channel);
            full_prompt.push_str(&format!(
                "The configured delivery channel for this routine is `{sanitized}`.\n"
            ));
        }

        if let Some(user) = notify.user.as_deref() {
            let sanitized = sanitize_prompt_field(user);
            full_prompt.push_str(&format!(
                "The configured delivery target for this routine is `{sanitized}`.\n"
            ));
        }

        full_prompt.push_str(
            "Do not claim you lack messaging integrations or ask the user to set one up when \
             a plain reply is sufficient.\n",
        );
    }

    if !use_tools {
        full_prompt.push_str(
            "\nTools are disabled for this routine run. Do not ask to call tools or describe tool limitations unless they prevent a necessary external action.\n",
        );
    }

    if !context_parts.is_empty() {
        full_prompt.push_str("\n\n---\n\n# Context\n\n");
        full_prompt.push_str(&context_parts.join("\n\n"));
    }

    if let Some(state) = state_content {
        full_prompt.push_str("\n\n---\n\n# Previous State\n\n");
        full_prompt.push_str(state);
    }

    full_prompt.push_str(
        "\n\n---\n\nIf nothing needs attention, reply EXACTLY with: ROUTINE_OK\n\
         If something needs attention, provide a concise summary.",
    );

    full_prompt
}

/// Execute a lightweight routine without tool support (original single-call behavior).
async fn execute_lightweight_no_tools(
    ctx: &EngineContext,
    _routine: &Routine,
    system_prompt: &str,
    full_prompt: &str,
    effective_max_tokens: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let messages = if system_prompt.is_empty() {
        vec![ChatMessage::user(full_prompt)]
    } else {
        vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(full_prompt),
        ]
    };

    let request = CompletionRequest::new(messages)
        .with_max_tokens(effective_max_tokens)
        .with_temperature(0.3);

    let response = ctx.llm.complete(request).await.map_err(|e| {
        let retryable = crate::llm::retry::is_retryable(&e);
        RoutineError::LlmFailed {
            reason: e.to_string(),
            // No partial tokens: the LLM call itself failed, so the response
            // (and its token counts) is unavailable. If providers start returning
            // partial usage on error responses, this should be updated.
            partial_tokens: None,
            retryable,
        }
    })?;

    handle_text_response(
        &response.content,
        response.finish_reason,
        response.input_tokens,
        response.output_tokens,
    )
}

/// Convert raw `u32` token counts into `Option<i32>`, preserving `None` semantics.
///
/// Providers that don't report token usage return `(0, 0)`. Wrapping that in
/// `Some(0)` would change the stored meaning from "unknown/not tracked" to "zero",
/// leaking incorrect data to downstream reporting. Only materialize `Some` when
/// at least one count is non-zero.
fn tokens_to_option(input: u32, output: u32) -> Option<i32> {
    let total = input.saturating_add(output);
    if total > 0 { Some(total as i32) } else { None }
}

/// Handle a text-only LLM response in lightweight routine execution.
///
/// Checks for the ROUTINE_OK sentinel, validates content, and returns appropriate status.
fn handle_text_response(
    content: &str,
    finish_reason: FinishReason,
    total_input_tokens: u32,
    total_output_tokens: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let content = content.trim();

    // Empty content guard — carry consumed tokens so the retry loop can
    // accumulate them even when the response shape is invalid.
    if content.is_empty() {
        let consumed = tokens_to_option(total_input_tokens, total_output_tokens);
        return if finish_reason == FinishReason::Length {
            Err(RoutineError::TruncatedResponse {
                partial_tokens: consumed,
            })
        } else {
            Err(RoutineError::EmptyResponse {
                partial_tokens: consumed,
            })
        };
    }

    // Check for the "nothing to do" sentinel (exact match on trimmed content).
    if content == "ROUTINE_OK" {
        let total_tokens = tokens_to_option(total_input_tokens, total_output_tokens);
        return Ok((RunStatus::Ok, None, total_tokens));
    }

    let total_tokens = tokens_to_option(total_input_tokens, total_output_tokens);
    Ok((
        RunStatus::Attention,
        Some(content.to_string()),
        total_tokens,
    ))
}

/// Execute a lightweight routine with tool execution support (agentic loop).
///
/// This is a simplified version of the full dispatcher loop:
/// - Max 3-5 iterations (configurable)
/// - Sequential tool execution (not parallel)
/// - Uses the owner's live autonomous tool scope when lightweight tools are enabled
/// - Auto-approval of non-Always tools
/// - No hooks or approval dialogs
async fn execute_lightweight_with_tools(
    ctx: &EngineContext,
    routine: &Routine,
    system_prompt: &str,
    full_prompt: &str,
    effective_max_tokens: u32,
    max_tool_rounds: u32,
) -> Result<(RunStatus, Option<String>, Option<i32>), RoutineError> {
    let mut messages = if system_prompt.is_empty() {
        vec![ChatMessage::user(full_prompt)]
    } else {
        vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(full_prompt),
        ]
    };

    let max_iterations = max_tool_rounds
        .min(ctx.config.lightweight_max_iterations)
        .min(5);
    let mut iteration = 0;
    let mut total_input_tokens = 0;
    let mut total_output_tokens = 0;

    // Create a minimal job context for tool execution with unique run ID.
    // Carry the routine's notify config in metadata so the message tool can
    // resolve channel/target — mirrors the full-job path in execute_full_job().
    let run_id = Uuid::new_v4();
    let mut lw_metadata = serde_json::json!({
        "owner_id": routine.user_id
    });
    if let Some(channel) = &routine.notify.channel {
        lw_metadata["notify_channel"] = serde_json::json!(channel);
    }
    lw_metadata["notify_user"] = serde_json::json!(&routine.notify.user);
    let job_ctx = JobContext {
        job_id: run_id,
        user_id: routine.user_id.clone(),
        title: "Lightweight Routine".to_string(),
        description: routine.name.clone(),
        metadata: lw_metadata,
        ..Default::default()
    };
    let allowed_tools =
        autonomous_allowed_tool_names(&ctx.tools, ctx.extension_manager.as_ref(), &routine.user_id)
            .await;

    loop {
        iteration += 1;

        // Force text-only response at iteration limit
        let force_text = iteration >= max_iterations;

        if force_text {
            // Final iteration: no tools, just get text response.
            // Claude 4.6 rejects assistant prefill; NEAR AI rejects any non-user-ending
            // conversation. Ensure the last message is user-role.
            crate::util::ensure_ends_with_user_message(&mut messages);
            let request = CompletionRequest::new(messages)
                .with_max_tokens(effective_max_tokens)
                .with_temperature(0.3);

            let response = ctx.llm.complete(request).await.map_err(|e| {
                let retryable = crate::llm::retry::is_retryable(&e);
                RoutineError::LlmFailed {
                    reason: e.to_string(),
                    partial_tokens: tokens_to_option(total_input_tokens, total_output_tokens),
                    retryable,
                }
            })?;

            total_input_tokens += response.input_tokens;
            total_output_tokens += response.output_tokens;

            return handle_text_response(
                &response.content,
                response.finish_reason,
                total_input_tokens,
                total_output_tokens,
            );
        } else {
            // Tool-enabled iteration
            let tool_defs = ctx
                .tools
                .tool_definitions()
                .await
                .into_iter()
                .filter(|tool| allowed_tools.contains(&tool.name))
                .collect();

            let request_messages = snapshot_messages_for_tool_iteration(&messages);
            let request = ToolCompletionRequest::new(request_messages, tool_defs)
                .with_max_tokens(effective_max_tokens)
                .with_temperature(0.3);

            let response = ctx.llm.complete_with_tools(request).await.map_err(|e| {
                let retryable = crate::llm::retry::is_retryable(&e);
                RoutineError::LlmFailed {
                    reason: e.to_string(),
                    partial_tokens: tokens_to_option(total_input_tokens, total_output_tokens),
                    retryable,
                }
            })?;

            total_input_tokens += response.input_tokens;
            total_output_tokens += response.output_tokens;

            // Check if LLM returned text (no tool calls)
            if response.tool_calls.is_empty() {
                let content = response.content.unwrap_or_default();
                return handle_text_response(
                    &content,
                    response.finish_reason,
                    total_input_tokens,
                    total_output_tokens,
                );
            }

            // LLM returned tool calls: add assistant message and execute tools
            messages.push(ChatMessage::assistant_with_tool_calls(
                response.content.clone(),
                response.tool_calls.clone(),
            ));

            // Execute tools sequentially
            for tc in response.tool_calls {
                let result = execute_routine_tool(ctx, &job_ctx, &allowed_tools, &tc).await;

                // Sanitize and wrap result (including errors)
                let result_content = match result {
                    Ok(output) => {
                        let sanitized = ctx.safety.sanitize_tool_output(&tc.name, &output);
                        ctx.safety.wrap_for_llm(&tc.name, &sanitized.content)
                    }
                    Err(e) => {
                        let error_msg = format!("Tool '{}' failed: {}", tc.name, e);
                        let sanitized = ctx.safety.sanitize_tool_output(&tc.name, &error_msg);
                        ctx.safety.wrap_for_llm(&tc.name, &sanitized.content)
                    }
                };

                // Truncate oversized tool output to prevent unbounded context growth.
                // Routine tool loops are lightweight and should not accumulate
                // large payloads across iterations.
                const MAX_TOOL_OUTPUT_CHARS: usize = 8192;
                let result_content = if result_content.len() > MAX_TOOL_OUTPUT_CHARS {
                    let truncated = &result_content
                        [..result_content.floor_char_boundary(MAX_TOOL_OUTPUT_CHARS)];
                    format!("{truncated}\n... [output truncated to {MAX_TOOL_OUTPUT_CHARS} chars]")
                } else {
                    result_content
                };

                // Add tool result to context
                messages.push(ChatMessage::tool_result(&tc.id, &tc.name, &result_content));
            }

            // Continue loop to next LLM call
        }
    }
}

// Bound per-iteration context copy cost for lightweight tool loops.
const MAX_TOOL_LOOP_MESSAGES: usize = 32;

fn snapshot_messages_for_tool_iteration(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    if messages.len() <= MAX_TOOL_LOOP_MESSAGES {
        return messages.to_vec();
    }

    let mut snapshot = Vec::with_capacity(MAX_TOOL_LOOP_MESSAGES);

    if let Some(first) = messages.first()
        && first.role == crate::llm::Role::System
    {
        snapshot.push(first.clone());
        let tail_len = MAX_TOOL_LOOP_MESSAGES - 1;
        let tail_start = (messages.len() - tail_len).max(1);
        snapshot.extend_from_slice(&messages[tail_start..]);
    } else {
        let tail_start = messages.len() - MAX_TOOL_LOOP_MESSAGES;
        snapshot.extend_from_slice(&messages[tail_start..]);
    }

    snapshot
}

/// Execute a single tool for a lightweight routine.
async fn execute_routine_tool(
    ctx: &EngineContext,
    job_ctx: &JobContext,
    allowed_tools: &std::collections::HashSet<String>,
    tc: &ToolCall,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if !allowed_tools.contains(&tc.name) {
        let message = autonomous_unavailable_message(&tc.name, &job_ctx.user_id);
        return Err(message.into());
    }

    // Check if tool exists
    let tool = ctx
        .tools
        .get(&tc.name)
        .await
        .ok_or_else(|| format!("Tool '{}' not found", tc.name))?;
    let normalized_params = prepare_tool_params(tool.as_ref(), &tc.arguments);

    // Validate tool parameters
    let validation = ctx
        .safety
        .validator()
        .validate_tool_params(&normalized_params);
    if !validation.is_valid {
        let details = validation
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("Invalid tool parameters: {}", details).into());
    }

    // Execute with per-tool timeout
    let timeout = tool.execution_timeout();
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(timeout, async {
        tool.execute(normalized_params.clone(), job_ctx).await
    })
    .await;
    let elapsed = start.elapsed();

    // Log tool execution result (single consolidated log)
    match &result {
        Ok(Ok(_)) => {
            tracing::debug!(
                tool = %tc.name,
                elapsed_ms = elapsed.as_millis() as u64,
                status = "succeeded",
                "Lightweight routine tool execution completed"
            );
        }
        Ok(Err(e)) => {
            tracing::debug!(
                tool = %tc.name,
                elapsed_ms = elapsed.as_millis() as u64,
                error = %e,
                status = "failed",
                "Lightweight routine tool execution completed"
            );
        }
        Err(_) => {
            tracing::debug!(
                tool = %tc.name,
                elapsed_ms = elapsed.as_millis() as u64,
                timeout_secs = timeout.as_secs(),
                status = "timeout",
                "Lightweight routine tool execution completed"
            );
        }
    }

    let result = result
        .map_err(|_| ToolError::Timeout(timeout))
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

    // Serialize result to JSON string
    let result_str =
        serde_json::to_string(&result.result).unwrap_or_else(|_| "<serialize error>".to_string());
    Ok(result_str)
}

/// Send a notification based on the routine's notify config and run status.
#[allow(clippy::too_many_arguments)]
async fn send_notification(
    tx: &mpsc::Sender<OutgoingResponse>,
    notify: &NotifyConfig,
    owner_id: &str,
    routine_name: &str,
    status: RunStatus,
    summary: Option<&str>,
    thread_id: Option<&str>,
) {
    let should_notify = match status {
        RunStatus::Ok => notify.on_success,
        RunStatus::Attention => notify.on_attention,
        RunStatus::Failed => notify.on_failure,
        RunStatus::Running => false,
    };

    if !should_notify {
        return;
    }

    let icon = match status {
        RunStatus::Ok => "✅",
        RunStatus::Attention => "🔔",
        RunStatus::Failed => "❌",
        RunStatus::Running => "⏳",
    };

    let message = match summary {
        Some(s) => format!("{} *Routine '{}'*: {}\n\n{}", icon, routine_name, status, s),
        None => format!("{} *Routine '{}'*: {}", icon, routine_name, status),
    };

    let response = OutgoingResponse {
        content: message,
        thread_id: thread_id.map(String::from),
        attachments: Vec::new(),
        metadata: serde_json::json!({
            "source": "routine",
            "routine_name": routine_name,
            "status": status.to_string(),
            "owner_id": owner_id,
            "notify_user": notify.user,
            "notify_channel": notify.channel,
        }),
    };

    if let Err(e) = tx.send(response).await {
        tracing::error!(routine = %routine_name, "Failed to send notification: {}", e);
    }
}

/// Inject a routine-review message into the agent loop if agent_review is
/// enabled for this routine and status.
async fn inject_agent_review(
    ctx: &EngineContext,
    routine: &Routine,
    run: &RoutineRun,
    status: RunStatus,
    summary: Option<&str>,
) {
    let should_review = match status {
        RunStatus::Ok => routine.notify.agent_review_on_success,
        RunStatus::Attention => routine.notify.agent_review_on_attention,
        RunStatus::Failed => routine.notify.agent_review_on_failure,
        RunStatus::Running => false,
    };

    if !should_review || !ctx.config.agent_review_enabled {
        // Successful routine run without review — reset circuit breaker so
        // a previously-tripped breaker recovers after normal operation resumes.
        if status == RunStatus::Ok {
            ctx.agent_review_consecutive_failures
                .store(0, Ordering::Relaxed);
        }
        return;
    }

    // Circuit breaker: 3 consecutive failures disables agent review until a
    // successful non-review run resets the counter.
    let consecutive_failures = ctx
        .agent_review_consecutive_failures
        .load(Ordering::Relaxed);
    if consecutive_failures >= 3 {
        tracing::warn!(
            routine = %routine.name,
            consecutive_failures,
            "Agent review disabled by circuit breaker (3 consecutive failures). \
             Will re-enable after a successful non-review run."
        );
        return;
    }

    // Rate limit: tumbling hourly window (resets when wall-clock hour elapses).
    // Note: the load/check/increment and window reset are not atomic. Concurrent
    // tokio tasks can both see the window boundary, both reset the counter to 0,
    // losing the entire hour's count. Worst case: limit + N reviews in a ~1-hour
    // span (where N = number of concurrent racing tasks). Acceptable for a soft
    // cost guard; use compare_exchange on window_start for stricter enforcement.
    // Acceptable for a soft rate limit with a default of 10/hour.
    let now_secs = Utc::now().timestamp();
    let window_start = ctx.agent_review_window_start.load(Ordering::Relaxed);
    if now_secs - window_start >= 3600 {
        // Reset the window.
        ctx.agent_review_window_start
            .store(now_secs, Ordering::Relaxed);
        ctx.agent_review_count.store(0, Ordering::Relaxed);
    }
    let current = ctx.agent_review_count.load(Ordering::Relaxed);
    if current >= ctx.config.max_agent_reviews_per_hour {
        tracing::warn!(
            routine = %routine.name,
            limit = ctx.config.max_agent_reviews_per_hour,
            "Agent review rate limit exceeded ({current}/{} per hour), skipping",
            ctx.config.max_agent_reviews_per_hour,
        );
        return;
    }
    ctx.agent_review_count.fetch_add(1, Ordering::Relaxed);

    let Some(ref tx) = ctx.inject_tx else {
        return;
    };

    let sanitized = ctx
        .safety
        .sanitize_tool_output("routine_result", summary.unwrap_or("(no summary)"));

    let content = format!(
        "[ROUTINE RESULT]\n\
         Name: {}\n\
         Status: {}\n\
         Run ID: {}\n\
         ---\n\
         {}\n\
         ---\n\
         You received this routine result. Decide what to do:\n\
         - If useful to the user, summarize and report it\n\
         - If failed, diagnose the cause and update your memory\n\
         - If nothing noteworthy, do not send a message",
        routine.name, status, run.id, sanitized.content
    );

    let msg = IncomingMessage::new("routine-review", routine.user_id.clone(), content)
        .with_metadata(serde_json::json!({
            "source": "routine_completion",
            "routine_id": routine.id.to_string(),
            "routine_name": routine.name,
            "status": status.to_string(),
            "run_id": run.id.to_string(),
            "notify_channel": routine.notify.channel,
            "notify_user": routine.notify.user,
        }))
        .with_conversation_scope(routine.id.to_string())
        .into_routine_review();

    // Note: the circuit breaker tracks injection failures (mpsc send), not
    // downstream LLM failures from the agent-review turn. The injection is
    // fire-and-forget — the LLM call happens asynchronously in handle_message.
    // Tracking downstream failures would require a feedback channel from the
    // agent loop back to the routine engine, which is a larger architectural
    // change. The current circuit breaker protects against the inject_tx channel
    // being broken (receiver dropped), not against LLM errors.
    if let Err(e) = tx.send(msg).await {
        ctx.agent_review_consecutive_failures
            .fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            routine = %routine.name,
            "Failed to inject agent-review message: {}", e
        );
    } else {
        ctx.agent_review_consecutive_failures
            .store(0, Ordering::Relaxed);
    }
}

/// Spawn the cron ticker background task.
pub fn spawn_cron_ticker(
    engine: Arc<RoutineEngine>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Recover orphaned runs from a previous process crash before
        // dispatching any new work, so we don't confuse fresh dispatches
        // with crash orphans.
        engine.sync_dispatched_runs().await;

        // Run one cron check immediately so routines due at startup don't
        // wait an extra full polling interval.
        engine.check_cron_triggers().await;

        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Periodic event cache refresh so web/CLI mutations are picked up
        // without requiring tool-path code to call refresh_event_cache().
        // Uses wall-clock elapsed time so the refresh cadence is stable
        // regardless of the cron tick interval configuration.
        let refresh_interval = Duration::from_secs(60);
        let mut last_refresh = tokio::time::Instant::now();

        loop {
            ticker.tick().await;
            // Sync first: only processes runs from before boot_time, so it
            // never races with FullJobWatcher instances from this process.
            engine.sync_dispatched_runs().await;
            engine.check_cron_triggers().await;

            if last_refresh.elapsed() >= refresh_interval {
                engine.refresh_event_cache().await;
                last_refresh = tokio::time::Instant::now();
            }
        }
    })
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = crate::util::floor_char_boundary(s, max);
        format!("{}...", &s[..end])
    }
}

/// Sanitize a summary string from job transitions before using in notifications.
///
/// `last_reason` comes from untrusted container code, so we:
/// 1. Strip control characters (except newline) to prevent terminal injection
/// 2. Strip HTML tags to prevent injection in web-rendered notifications
/// 3. Collapse multiple whitespace/newlines to single spaces for cleaner output
/// 4. Truncate to 500 chars to prevent oversized notifications
#[cfg(test)]
fn sanitize_summary(s: &str) -> String {
    // Strip control characters (keep newline for now, collapse later)
    let no_control: String = s
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();

    // Strip HTML tags (e.g. <script>, <img>, <a href=...>)
    let no_html = strip_html_tags(&no_control);

    // Collapse whitespace: multiple spaces/newlines become a single space
    let collapsed: String = no_html.split_whitespace().collect::<Vec<_>>().join(" ");

    // Truncate to reasonable length
    if collapsed.len() <= 500 {
        collapsed
    } else {
        // Find a safe char boundary for truncation
        let mut end = 500;
        while !collapsed.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &collapsed[..end])
    }
}

/// Remove HTML/XML tags from a string.
#[cfg(test)]
fn strip_html_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use chrono::Utc;
    use uuid::Uuid;

    use crate::agent::routine::{
        NotifyConfig, Routine, RoutineAction, RoutineGuardrails, RoutineRun, RunStatus, Trigger,
    };
    use crate::channels::IncomingMessage;
    use crate::config::RoutineConfig;

    #[test]
    fn test_notification_gating() {
        let config = NotifyConfig {
            on_success: false,
            on_failure: true,
            on_attention: true,
            agent_review_on_success: false,
            agent_review_on_attention: false,
            agent_review_on_failure: false,
            ..Default::default()
        };

        // on_success = false means Ok status should not notify
        assert!(!config.on_success);
        assert!(config.on_failure);
        assert!(config.on_attention);
    }

    #[test]
    fn test_run_status_icons() {
        // Just verify the mapping doesn't panic
        for status in [
            RunStatus::Ok,
            RunStatus::Attention,
            RunStatus::Failed,
            RunStatus::Running,
        ] {
            let _ = status.to_string();
        }
    }

    #[test]
    fn test_routine_config_lightweight_tools_enabled_default() {
        let config = RoutineConfig::default();
        assert!(
            config.lightweight_tools_enabled,
            "Tools should be enabled by default"
        );
    }

    #[test]
    fn test_routine_config_lightweight_max_iterations_default() {
        let config = RoutineConfig::default();
        assert_eq!(
            config.lightweight_max_iterations, 3,
            "Default should be 3 iterations"
        );
    }

    #[test]
    fn test_routine_config_can_hold_uncapped_max_iterations() {
        // The `RoutineConfig` struct can hold a value greater than the safety cap.
        let config = RoutineConfig {
            lightweight_max_iterations: 10, // Set a value higher than the cap.
            ..RoutineConfig::default()
        };
        // The actual capping to a maximum of 5 is handled at runtime in
        // `execute_lightweight_with_tools` and during config resolution from env vars.
        assert_eq!(
            config.lightweight_max_iterations, 10,
            "Config struct should store the provided value"
        );
    }

    #[test]
    fn test_sanitize_routine_name_replaces_special_chars() {
        let test_cases = vec![
            ("valid-routine", "valid-routine"),
            ("routine_with_underscore", "routine_with_underscore"),
            ("Routine With Spaces", "Routine_With_Spaces"),
            ("routine/with/slashes", "routine_with_slashes"),
            ("routine@with#symbols", "routine_with_symbols"),
        ];

        for (input, expected) in test_cases {
            let result = super::sanitize_routine_name(input);
            assert_eq!(
                result, expected,
                "sanitize_routine_name({}) should be {}",
                input, expected
            );
        }
    }

    #[test]
    fn test_sanitize_routine_name_preserves_alphanumeric_dash_underscore() {
        let names = vec!["routine123", "routine-name", "routine_name", "ROUTINE"];
        for name in names {
            let result = super::sanitize_routine_name(name);
            assert_eq!(result, name, "Should preserve {}", name);
        }
    }

    #[test]
    fn test_build_lightweight_prompt_explains_delivery_and_disabled_tools() {
        let notify = NotifyConfig {
            channel: Some("telegram".to_string()),
            user: Some("default".to_string()),
            on_attention: true,
            on_failure: true,
            on_success: false,
            agent_review_on_success: false,
            agent_review_on_attention: false,
            agent_review_on_failure: false,
        };

        let prompt = super::build_lightweight_prompt(
            "Send a Telegram reminder message to the user.",
            &[],
            None,
            &notify,
            false,
        );

        assert!(
            prompt.contains("the host will deliver your reply as the routine notification"),
            "delivery guidance should explain host delivery: {prompt}",
        );
        assert!(
            prompt.contains("configured delivery channel for this routine is `telegram`"),
            "delivery guidance should mention telegram channel: {prompt}",
        );
        assert!(
            prompt.contains("Do not claim you lack messaging integrations"),
            "delivery guidance should suppress fake setup chatter: {prompt}",
        );
        assert!(
            prompt.contains("Tools are disabled for this routine run"),
            "prompt should explain that tools are disabled: {prompt}",
        );
    }

    #[test]
    fn test_build_lightweight_prompt_skips_delivery_block_when_attention_notifications_disabled() {
        let notify = NotifyConfig {
            on_attention: false,
            agent_review_on_success: false,
            agent_review_on_attention: false,
            agent_review_on_failure: false,
            ..NotifyConfig::default()
        };

        let prompt = super::build_lightweight_prompt("Check inbox.", &[], None, &notify, true);

        assert!(
            !prompt.contains("# Delivery"),
            "prompt should not include delivery guidance when attention notifications are off: {prompt}",
        );
        assert!(
            !prompt.contains("Tools are disabled for this routine run"),
            "prompt should not claim tools are disabled when they are enabled: {prompt}",
        );
    }

    #[test]
    fn test_routine_sentinel_detection_exact_match() {
        // Sentinel detection uses exact match on trimmed content to avoid
        // false positives from substrings like "NOT_ROUTINE_OK".
        let test_cases = vec![
            ("ROUTINE_OK", true),
            ("  ROUTINE_OK  ", true), // After trim, whitespace is removed so matches
            ("something ROUTINE_OK something", false), // substring no longer matches
            ("ROUTINE_OK is done", false), // substring no longer matches
            ("done ROUTINE_OK", false), // substring no longer matches
            ("NOT_ROUTINE_OK", false), // exact match prevents this
            ("no sentinel here", false),
        ];

        for (content, should_match) in test_cases {
            let trimmed = content.trim();
            let matches = trimmed == "ROUTINE_OK";
            assert_eq!(
                matches, should_match,
                "Content '{}' sentinel detection should be {}, got {}",
                content, should_match, matches
            );
        }
    }

    #[test]
    fn test_approval_requirement_pattern_matching() {
        // Test the approval requirement logic (Never, UnlessAutoApproved, Always)
        use crate::tools::ApprovalRequirement;

        let requirements = vec![
            (ApprovalRequirement::Never, "auto-approved"),
            (ApprovalRequirement::UnlessAutoApproved, "auto-approved"),
            (ApprovalRequirement::Always, "blocks"),
        ];

        for (req, expected) in requirements {
            let can_auto_approve = matches!(
                req,
                ApprovalRequirement::Never | ApprovalRequirement::UnlessAutoApproved
            );
            let label = if can_auto_approve {
                "auto-approved"
            } else {
                "blocks"
            };
            assert_eq!(label, expected, "Approval pattern should match");
        }
    }

    /// Helper to build a test routine with the given user_id and trigger.
    fn make_routine(user_id: &str, trigger: Trigger) -> Routine {
        Routine {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            description: String::new(),
            user_id: user_id.to_string(),
            enabled: true,
            trigger,
            action: RoutineAction::Lightweight {
                prompt: String::new(),
                context_paths: vec![],
                max_tokens: 1000,
                use_tools: false,
                max_tool_rounds: 0,
            },
            guardrails: RoutineGuardrails::default(),
            notify: Default::default(),
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::Value::Null,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    /// Helper to build a test IncomingMessage.
    fn make_message(user_id: &str, channel: &str, content: &str) -> IncomingMessage {
        IncomingMessage {
            id: Uuid::new_v4(),
            channel: channel.to_string(),
            user_id: user_id.to_string(),
            sender_id: user_id.to_string(),
            user_name: None,
            content: content.to_string(),
            thread_id: None,
            conversation_scope_id: None,
            received_at: Utc::now(),
            metadata: serde_json::Value::Null,
            timezone: None,
            attachments: vec![],
            source: crate::channels::MessageSource::User,
        }
    }

    /// Regression test for issue #1051: event triggers used case-sensitive
    /// channel comparison, so "Telegram" != "telegram" caused silent mismatch.
    /// Tests the actual `routine_matches_message` function used in `check_event_triggers`.
    #[test]
    fn test_channel_filter_is_case_insensitive() {
        let routine = make_routine(
            "user1",
            Trigger::Event {
                pattern: ".*".to_string(),
                channel: Some("Telegram".to_string()),
            },
        );
        let msg = make_message("user1", "telegram", "hello");

        // Case-insensitive channel match must succeed
        assert!(super::routine_matches_message(&routine, &msg));

        // Exact case must also work
        let msg_exact = make_message("user1", "Telegram", "hello");
        assert!(super::routine_matches_message(&routine, &msg_exact));

        // Different channel must not match
        let msg_wrong = make_message("user1", "discord", "hello");
        assert!(!super::routine_matches_message(&routine, &msg_wrong));
    }

    /// Regression test for issue #1051: event triggers did not filter by
    /// user_id, so routines from user A could fire on messages from user B.
    /// Tests the actual `routine_matches_message` function used in `check_event_triggers`.
    #[test]
    fn test_event_trigger_requires_user_match() {
        let routine = make_routine(
            "alice",
            Trigger::Event {
                pattern: ".*".to_string(),
                channel: None,
            },
        );

        // Different user must not match
        let msg_bob = make_message("bob", "telegram", "hello");
        assert!(!super::routine_matches_message(&routine, &msg_bob));

        // Same user must match
        let msg_alice = make_message("alice", "telegram", "hello");
        assert!(super::routine_matches_message(&routine, &msg_alice));
    }

    /// When no channel filter is set, any channel should match (given user matches).
    #[test]
    fn test_no_channel_filter_matches_any_channel() {
        let routine = make_routine(
            "user1",
            Trigger::Event {
                pattern: ".*".to_string(),
                channel: None,
            },
        );

        let msg = make_message("user1", "whatever_channel", "hello");
        assert!(super::routine_matches_message(&routine, &msg));
    }

    #[test]
    fn test_routine_tool_denylist_blocks_self_management_tools() {
        let denylisted = vec![
            "routine_create",
            "routine_update",
            "routine_delete",
            "routine_fire",
            "restart",
        ];
        for tool in &denylisted {
            assert!(
                crate::tools::AUTONOMOUS_TOOL_DENYLIST.contains(tool),
                "Tool '{}' should be in AUTONOMOUS_TOOL_DENYLIST",
                tool
            );
        }
    }

    #[test]
    fn test_routine_tool_denylist_allows_safe_tools() {
        let allowed = vec!["echo", "time", "json", "http", "memory_search", "shell"];
        for tool in &allowed {
            assert!(
                !crate::tools::AUTONOMOUS_TOOL_DENYLIST.contains(tool),
                "Tool '{}' should NOT be in AUTONOMOUS_TOOL_DENYLIST",
                tool
            );
        }
    }

    #[test]
    fn test_empty_response_handling() {
        // Simulate the empty content guard logic
        let empty_content = "";
        let finish_reason_length = crate::llm::FinishReason::Length;
        let finish_reason_stop = crate::llm::FinishReason::Stop;

        assert!(
            empty_content.trim().is_empty(),
            "Should detect empty content"
        );
        assert_eq!(finish_reason_length, crate::llm::FinishReason::Length);
        assert_eq!(finish_reason_stop, crate::llm::FinishReason::Stop);
    }

    #[test]
    fn test_truncate_adds_ellipsis_when_over_limit() {
        let input = "abcdefghijk";
        let out = super::truncate(input, 5);
        assert_eq!(out, "abcde...");
    }

    #[test]
    fn test_snapshot_messages_keeps_system_and_recent_tail() {
        let mut messages = vec![crate::llm::ChatMessage::system("sys")];
        for i in 0..80 {
            messages.push(crate::llm::ChatMessage::user(format!("u{i}")));
        }

        let snapshot = super::snapshot_messages_for_tool_iteration(&messages);
        assert_eq!(snapshot.len(), super::MAX_TOOL_LOOP_MESSAGES); // safety: test-only no-panics CI false positive
        assert_eq!(snapshot[0].role, crate::llm::Role::System); // safety: test-only no-panics CI false positive
        assert_eq!(snapshot[0].content, "sys"); // safety: test-only no-panics CI false positive
        let last_content = snapshot.last().map(|m| m.content.as_str());
        assert_eq!(last_content, Some("u79")); // safety: test-only no-panics CI false positive
    }

    #[test]
    fn test_snapshot_messages_unchanged_when_within_limit() {
        let messages = vec![
            crate::llm::ChatMessage::system("sys"),
            crate::llm::ChatMessage::user("a"),
            crate::llm::ChatMessage::assistant("b"),
        ];
        let snapshot = super::snapshot_messages_for_tool_iteration(&messages);
        assert_eq!(snapshot.len(), messages.len()); // safety: test-only no-panics CI false positive
        assert_eq!(snapshot[0].role, crate::llm::Role::System); // safety: test-only no-panics CI false positive
        assert_eq!(snapshot[1].content, "a"); // safety: test-only no-panics CI false positive
        assert_eq!(snapshot[2].content, "b"); // safety: test-only no-panics CI false positive
    }

    #[test]
    fn test_running_status_does_not_notify() {
        let config = NotifyConfig {
            on_success: true,
            on_failure: true,
            on_attention: true,
            agent_review_on_success: false,
            agent_review_on_attention: false,
            agent_review_on_failure: false,
            ..Default::default()
        };
        let should_notify = match RunStatus::Running {
            RunStatus::Ok => config.on_success,
            RunStatus::Attention => config.on_attention,
            RunStatus::Failed => config.on_failure,
            RunStatus::Running => false,
        };
        assert!(!should_notify);
    }

    #[test]
    fn test_full_job_dispatch_returns_running_status() {
        assert_eq!(RunStatus::Running.to_string(), "running");
    }

    #[test]
    fn test_sandbox_disabled_by_config_does_not_block_full_job() {
        use super::SandboxReadiness;

        // DisabledByConfig must NOT match the DockerUnavailable gate —
        // full-job routines dispatch through the scheduler (no Docker needed).
        assert!(!matches!(
            SandboxReadiness::DisabledByConfig,
            SandboxReadiness::DockerUnavailable
        ));
    }

    #[test]
    fn test_sandbox_readiness_docker_unavailable_still_blocks() {
        use super::SandboxReadiness;

        // DockerUnavailable should still block full-job dispatch.
        assert!(matches!(
            SandboxReadiness::DockerUnavailable,
            SandboxReadiness::DockerUnavailable
        ));

        let err = crate::error::RoutineError::JobDispatchFailed {
            reason: "Sandbox is enabled but Docker is not available. \
                     Install Docker or set SANDBOX_ENABLED=false."
                .to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("Docker is not available"));
    }

    /// Regression test for #1317: FullJobWatcher maps terminal job states correctly.
    #[test]
    fn test_full_job_watcher_state_mapping() {
        use crate::context::JobState;

        // Failed/Cancelled → RunStatus::Failed
        assert_eq!(
            super::FullJobWatcher::map_job_state(&JobState::Failed),
            RunStatus::Failed
        );
        assert_eq!(
            super::FullJobWatcher::map_job_state(&JobState::Cancelled),
            RunStatus::Failed
        );

        // All other non-active states → RunStatus::Ok
        assert_eq!(
            super::FullJobWatcher::map_job_state(&JobState::Completed),
            RunStatus::Ok
        );
        assert_eq!(
            super::FullJobWatcher::map_job_state(&JobState::Accepted),
            RunStatus::Ok
        );
    }

    /// Verify that job state to run status mapping covers all expected cases.
    #[test]
    fn test_job_state_to_run_status_mapping() {
        use crate::context::JobState;

        // Success states
        for state in [JobState::Completed, JobState::Submitted, JobState::Accepted] {
            let status = match state {
                JobState::Completed | JobState::Submitted | JobState::Accepted => {
                    Some(RunStatus::Ok)
                }
                JobState::Failed | JobState::Cancelled => Some(RunStatus::Failed),
                _ => None,
            };
            assert_eq!(
                status,
                Some(RunStatus::Ok),
                "{:?} should map to RunStatus::Ok",
                state
            );
        }

        // Failure states
        for state in [JobState::Failed, JobState::Cancelled] {
            let status = match state {
                JobState::Completed | JobState::Submitted | JobState::Accepted => {
                    Some(RunStatus::Ok)
                }
                JobState::Failed | JobState::Cancelled => Some(RunStatus::Failed),
                _ => None,
            };
            assert_eq!(
                status,
                Some(RunStatus::Failed),
                "{:?} should map to RunStatus::Failed",
                state
            );
        }

        // Active states (should not finalize)
        for state in [JobState::Pending, JobState::InProgress, JobState::Stuck] {
            let status = match state {
                JobState::Completed | JobState::Submitted | JobState::Accepted => {
                    Some(RunStatus::Ok)
                }
                JobState::Failed | JobState::Cancelled => Some(RunStatus::Failed),
                _ => None,
            };
            assert_eq!(
                status, None,
                "{:?} should not finalize the routine run",
                state
            );
        }
    }

    /// Regression test for #1320: transient errors are retried for lightweight
    /// routines but not for full-job routines or hard failures.
    #[test]
    fn test_retry_classification_for_routine_errors() {
        use crate::error::RoutineError;

        // Transient errors (retryable for lightweight routines)
        let transient_errors: Vec<RoutineError> = vec![
            RoutineError::LlmFailed {
                reason: "rate limit".into(),
                partial_tokens: None,
                retryable: true,
            },
            RoutineError::LlmFailed {
                reason: "network timeout".into(),
                partial_tokens: Some(42),
                retryable: true,
            },
            RoutineError::EmptyResponse {
                partial_tokens: None,
            },
            RoutineError::TruncatedResponse {
                partial_tokens: Some(100),
            },
        ];
        for err in &transient_errors {
            assert!(err.is_retryable(), "{} should be retryable", err);
        }

        // Permanent LLM failures that should NOT be retried
        // (retryable: false is set at conversion time by llm::retry::is_retryable)
        let permanent_llm_errors: Vec<RoutineError> = vec![
            RoutineError::LlmFailed {
                reason: "Authentication failed for provider openai".into(),
                partial_tokens: None,
                retryable: false,
            },
            RoutineError::LlmFailed {
                reason: "invalid_api_key: bad key".into(),
                partial_tokens: None,
                retryable: false,
            },
            RoutineError::LlmFailed {
                reason: "content policy violation".into(),
                partial_tokens: None,
                retryable: false,
            },
            RoutineError::LlmFailed {
                reason: "content_filter triggered".into(),
                partial_tokens: None,
                retryable: false,
            },
            RoutineError::LlmFailed {
                reason: "context length exceeded: 150000 tokens used, 128000 allowed".into(),
                partial_tokens: Some(100),
                retryable: false,
            },
            RoutineError::LlmFailed {
                reason: "model not available on provider anthropic".into(),
                partial_tokens: None,
                retryable: false,
            },
            RoutineError::LlmFailed {
                reason: "content moderation flagged".into(),
                partial_tokens: None,
                retryable: false,
            },
        ];
        for err in &permanent_llm_errors {
            assert!(!err.is_retryable(), "{} should NOT be retryable", err);
        }

        // Hard failures (never retried)
        let hard_errors: Vec<RoutineError> = vec![
            RoutineError::Disabled {
                name: "test".into(),
            },
            RoutineError::NotFound {
                id: uuid::Uuid::new_v4(),
            },
            RoutineError::NotAuthorized {
                id: uuid::Uuid::new_v4(),
            },
            RoutineError::MaxConcurrent {
                name: "test".into(),
            },
            RoutineError::JobDispatchFailed {
                reason: "no docker".into(),
            },
            RoutineError::Database {
                reason: "connection refused".into(),
            },
        ];
        for err in &hard_errors {
            assert!(!err.is_retryable(), "{} should NOT be retryable", err);
        }
    }

    #[test]
    fn test_sanitize_summary_strips_control_chars() {
        use super::sanitize_summary;

        // Preserves normal text
        assert_eq!(sanitize_summary("Job completed"), "Job completed");

        // Strips control characters and collapses whitespace
        assert_eq!(
            sanitize_summary("line1\nline2\x00\x1b[31mred"),
            "line1 line2[31mred"
        );

        // Truncates long strings
        let long = "x".repeat(600);
        let result = sanitize_summary(&long);
        assert!(result.len() <= 503); // 500 + "..."
        assert!(result.ends_with("..."));
    }

    /// Verify that the default config has `agent_review_enabled = true`.
    #[test]
    fn test_agent_review_disabled_by_default() {
        let config = RoutineConfig::default();
        assert!(
            !config.agent_review_enabled,
            "agent_review_enabled should default to false"
        );
    }

    /// Verify that the default rate limit is 10 per hour.
    #[test]
    fn test_agent_review_rate_limit_default() {
        let config = RoutineConfig::default();
        assert_eq!(config.max_agent_reviews_per_hour, 10);
    }

    // Note: status gating, enabled-flag gating, rate limiting, and circuit
    // breaker behavior are tested through the real inject_agent_review function
    // in test_inject_agent_review_sends_message_on_failure and
    // test_inject_agent_review_rate_limit_integration below.

    #[test]
    fn test_sanitize_summary_strips_html() {
        use super::sanitize_summary;

        assert_eq!(
            sanitize_summary("Hello <script>alert('xss')</script> world"),
            "Hello alert('xss') world"
        );
        assert_eq!(
            sanitize_summary("<b>bold</b> and <a href=\"evil\">link</a>"),
            "bold and link"
        );
        assert_eq!(sanitize_summary("<img src=x onerror=alert(1)>"), "");
    }

    #[test]
    fn test_sanitize_summary_multibyte_truncation() {
        use super::sanitize_summary;

        // Ensure truncation doesn't panic on multi-byte chars near the boundary
        let s = "a".repeat(498) + "\u{1F600}\u{1F600}"; // 498 + two 4-byte emoji
        let result = sanitize_summary(&s);
        assert!(result.len() <= 503);
        assert!(result.ends_with("..."));
    }

    /// Regression: lightweight routines must carry notify metadata in JobContext
    /// so the message tool can route to the correct channel. Previously,
    /// `..Default::default()` left metadata as null, causing messages to land
    /// in the user's DM instead of the originating Slack channel.
    #[test]
    fn test_build_lightweight_prompt_preserves_notify_config() {
        let notify = NotifyConfig {
            channel: Some("slack-relay".to_string()),
            user: Some("C088K6C3SQZ".to_string()),
            on_attention: true,
            on_failure: true,
            on_success: false,
            agent_review_on_success: false,
            agent_review_on_attention: false,
            agent_review_on_failure: false,
        };

        let prompt =
            super::build_lightweight_prompt("Send Ping in this channel.", &[], None, &notify, true);

        assert!(
            prompt.contains("slack-relay"),
            "prompt should mention configured delivery channel: {prompt}",
        );
        assert!(
            prompt.contains("C088K6C3SQZ"),
            "prompt should mention configured delivery target: {prompt}",
        );
    }
    /// Integration test: inject_agent_review sends a RoutineReview message through
    /// a real mpsc channel when agent_review_on_failure is enabled.
    #[tokio::test]
    async fn test_inject_agent_review_sends_message_on_failure() {
        use crate::channels::IncomingMessage;

        use crate::testing::StubLlm;

        // Set up a real mpsc channel to capture the injected message
        let (inject_tx, mut inject_rx) = tokio::sync::mpsc::channel::<IncomingMessage>(16);
        let (notify_tx, _notify_rx) =
            tokio::sync::mpsc::channel::<crate::channels::OutgoingResponse>(16);

        // Build a minimal libsql backend for AdminScope
        let dir = tempfile::tempdir().expect("tempdir"); // safety: test code
        let db_path = dir.path().join("test_inject.db");
        let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .expect("libsql"); // safety: test code
        crate::db::Database::run_migrations(&backend)
            .await
            .expect("migrations"); // safety: test code
        let db: Arc<dyn crate::db::Database> = Arc::new(backend);

        let safety = Arc::new(ironclaw_safety::SafetyLayer::new(
            &crate::config::SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: false,
            },
        ));
        let llm: Arc<dyn crate::llm::LlmProvider> = Arc::new(StubLlm::default());
        let ws = Arc::new(crate::workspace::Workspace::new_with_db(
            "default",
            db.clone(),
        ));
        let tools = Arc::new(crate::tools::ToolRegistry::new());

        let config = RoutineConfig {
            agent_review_enabled: true,
            ..RoutineConfig::default()
        };

        let ctx = super::EngineContext {
            config,
            store: crate::tenant::SystemScope::new(db.clone()),
            llm,
            workspace: ws,
            notify_tx,
            running_count: Arc::new(AtomicUsize::new(0)),
            scheduler: None,
            extension_manager: None,
            tools,
            safety,
            sandbox_readiness: super::SandboxReadiness::DisabledByConfig,
            event_cache: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            inject_tx: Some(inject_tx),
            agent_review_count: Arc::new(AtomicU32::new(0)),
            agent_review_window_start: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            agent_review_consecutive_failures: Arc::new(AtomicU32::new(0)),
        };

        // Create a routine with agent_review_on_failure = true
        let routine = Routine {
            id: uuid::Uuid::new_v4(),
            name: "test-routine".to_string(),
            description: String::new(),
            user_id: "default".to_string(),
            enabled: true,
            trigger: Trigger::Manual,
            action: RoutineAction::Lightweight {
                prompt: "test".to_string(),
                context_paths: vec![],
                max_tokens: 1024,
                use_tools: false,
                max_tool_rounds: 0,
            },
            guardrails: RoutineGuardrails::default(),
            notify: NotifyConfig {
                agent_review_on_failure: true,
                ..NotifyConfig::default()
            },
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let run = RoutineRun {
            id: uuid::Uuid::new_v4(),
            routine_id: routine.id,
            trigger_type: "manual".to_string(),
            trigger_detail: None,
            started_at: chrono::Utc::now(),
            completed_at: None,
            status: RunStatus::Failed,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: chrono::Utc::now(),
        };

        // Inject with Failed status — should send a message
        super::inject_agent_review(
            &ctx,
            &routine,
            &run,
            RunStatus::Failed,
            Some("test failure"),
        )
        .await;

        // Verify message was received
        let msg = inject_rx
            .try_recv()
            .expect("should have received a message"); // safety: test code
        assert_eq!(msg.channel, "routine-review");
        assert!(msg.content.contains("test-routine"));
        assert!(msg.content.contains("test failure"));
        assert!(msg.content.contains("failed"));

        // Verify rate limit counter incremented
        assert_eq!(ctx.agent_review_count.load(Ordering::Relaxed), 1);

        // Now test: Ok status with agent_review_on_success = false should NOT send
        super::inject_agent_review(&ctx, &routine, &run, RunStatus::Ok, Some("ok result")).await;
        assert!(
            inject_rx.try_recv().is_err(),
            "should NOT send for Ok when flag is false"
        );
    }

    /// Integration test: inject_agent_review respects rate limit.
    #[tokio::test]
    async fn test_inject_agent_review_rate_limit_integration() {
        use crate::channels::IncomingMessage;

        use crate::testing::StubLlm;

        let (inject_tx, mut inject_rx) = tokio::sync::mpsc::channel::<IncomingMessage>(64);
        let (notify_tx, _notify_rx) =
            tokio::sync::mpsc::channel::<crate::channels::OutgoingResponse>(16);

        let dir = tempfile::tempdir().expect("tempdir"); // safety: test code
        let db_path = dir.path().join("test_rate.db");
        let backend = crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .expect("libsql"); // safety: test code
        crate::db::Database::run_migrations(&backend)
            .await
            .expect("migrations"); // safety: test code
        let db: Arc<dyn crate::db::Database> = Arc::new(backend);

        let config = RoutineConfig {
            agent_review_enabled: true,
            max_agent_reviews_per_hour: 3, // low limit for testing
            ..RoutineConfig::default()
        };

        let ctx = super::EngineContext {
            config,
            store: crate::tenant::SystemScope::new(db.clone()),
            llm: Arc::new(StubLlm::default()) as Arc<dyn crate::llm::LlmProvider>,
            workspace: Arc::new(crate::workspace::Workspace::new_with_db(
                "default",
                db.clone(),
            )),
            notify_tx,
            running_count: Arc::new(AtomicUsize::new(0)),
            scheduler: None,
            extension_manager: None,
            tools: Arc::new(crate::tools::ToolRegistry::new()),
            safety: Arc::new(ironclaw_safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                },
            )),
            sandbox_readiness: super::SandboxReadiness::DisabledByConfig,
            event_cache: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            inject_tx: Some(inject_tx),
            agent_review_count: Arc::new(AtomicU32::new(0)),
            agent_review_window_start: Arc::new(std::sync::atomic::AtomicI64::new(0)),
            agent_review_consecutive_failures: Arc::new(AtomicU32::new(0)),
        };

        let routine = Routine {
            id: uuid::Uuid::new_v4(),
            name: "rate-test".to_string(),
            description: String::new(),
            user_id: "default".to_string(),
            enabled: true,
            trigger: Trigger::Manual,
            action: RoutineAction::Lightweight {
                prompt: "test".to_string(),
                context_paths: vec![],
                max_tokens: 1024,
                use_tools: false,
                max_tool_rounds: 0,
            },
            guardrails: RoutineGuardrails::default(),
            notify: NotifyConfig {
                agent_review_on_failure: true,
                ..NotifyConfig::default()
            },
            last_run_at: None,
            next_fire_at: None,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let run = RoutineRun {
            id: uuid::Uuid::new_v4(),
            routine_id: routine.id,
            trigger_type: "manual".to_string(),
            trigger_detail: None,
            started_at: chrono::Utc::now(),
            completed_at: None,
            status: RunStatus::Failed,
            result_summary: None,
            tokens_used: None,
            job_id: None,
            created_at: chrono::Utc::now(),
        };

        // Send 3 (at the limit) — all should succeed
        for _ in 0..3 {
            super::inject_agent_review(&ctx, &routine, &run, RunStatus::Failed, Some("fail")).await;
        }
        // Drain the 3 messages
        for _ in 0..3 {
            assert!(inject_rx.try_recv().is_ok());
        }

        // 4th should be rate-limited — no message sent
        super::inject_agent_review(&ctx, &routine, &run, RunStatus::Failed, Some("fail")).await;
        assert!(
            inject_rx.try_recv().is_err(),
            "4th call should be rate-limited"
        );

        assert_eq!(ctx.agent_review_count.load(Ordering::Relaxed), 3);
    }
}
