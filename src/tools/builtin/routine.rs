//! LLM-facing tools for managing routines.
//!
//! Seven tools let the agent manage routines conversationally:
//! - `routine_create` - Create a new routine
//! - `routine_list` - List all routines with status
//! - `routine_update` - Modify or toggle a routine
//! - `routine_delete` - Remove a routine
//! - `routine_fire` - Manually trigger a routine
//! - `routine_history` - View past runs
//! - `event_emit` - Emit a structured system event to `system_event`-triggered routines

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger, next_cron_fire,
};
use crate::agent::routine_engine::RoutineEngine;
use crate::context::JobContext;
use crate::db::Database;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

// ==================== routine_create ====================

pub struct RoutineCreateTool {
    store: Arc<dyn Database>,
    engine: Arc<RoutineEngine>,
}

impl RoutineCreateTool {
    pub fn new(store: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Self {
        Self { store, engine }
    }
}

#[async_trait]
impl Tool for RoutineCreateTool {
    fn name(&self) -> &str {
        "routine_create"
    }

    fn description(&self) -> &str {
        "Create a new routine (scheduled or event-driven task). \
         Supports cron schedules, event pattern matching, system events, and manual triggers. \
         Use this when the user wants something to happen periodically or reactively."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Unique name for the routine (e.g. 'daily-pr-review')"
                },
                "description": {
                    "type": "string",
                    "description": "What this routine does"
                },
                "trigger_type": {
                    "type": "string",
                    "enum": ["cron", "event", "system_event", "manual"],
                    "description": "When the routine fires"
                },
                "schedule": {
                    "type": "string",
                    "description": "Cron expression (for cron trigger). E.g. '0 9 * * MON-FRI' for weekdays at 9am. Uses 6-field cron (sec min hour day month weekday)."
                },
                "event_pattern": {
                    "type": "string",
                    "description": "Regex pattern to match messages (for event trigger)"
                },
                "event_channel": {
                    "type": "string",
                    "description": "Optional channel filter for event trigger (e.g. 'telegram')"
                },
                "event_source": {
                    "type": "string",
                    "description": "Event source for system_event triggers (e.g. 'github')"
                },
                "event_type": {
                    "type": "string",
                    "description": "Event type for system_event triggers (e.g. 'issue.opened')"
                },
                "event_filters": {
                    "type": "object",
                    "description": "Optional exact-match filters against payload fields for system_event triggers. Values can be strings, numbers, or booleans."
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt/instructions for the routine"
                },
                "context_paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Workspace paths to load as context (e.g. ['context/priorities.md'])"
                },
                "action_type": {
                    "type": "string",
                    "enum": ["lightweight", "full_job"],
                    "description": "Execution mode: 'lightweight' (single LLM call, default) or 'full_job' (multi-turn with tools)"
                },
                "use_tools": {
                    "type": "boolean",
                    "description": "Enable tool access in lightweight mode (default: false). Only safe tools (no approval required) are available. Ignored for full_job mode."
                },
                "max_tool_rounds": {
                    "type": "integer",
                    "description": "Max tool call rounds in lightweight mode (default: 3). Only used when use_tools is true."
                },
                "cooldown_secs": {
                    "type": "integer",
                    "description": "Minimum seconds between fires (default: 300)"
                },
                "tool_permissions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tool names pre-authorized for Always-approval tools in full_job mode (e.g. ['shell']). UnlessAutoApproved tools are automatically permitted in routines."
                },
                "notify_channel": {
                    "type": "string",
                    "description": "Channel to send results to (e.g. 'telegram', 'slack', 'tui'). Sets the default channel for message tool calls in routine jobs."
                },
                "notify_user": {
                    "type": "string",
                    "description": "User/target to notify (e.g. username, chat ID). Defaults to 'default'."
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone for cron schedule evaluation (e.g. 'America/New_York'). Defaults to UTC."
                }
            },
            "required": ["name", "trigger_type", "prompt"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let trigger_type = require_str(&params, "trigger_type")?;

        let prompt = require_str(&params, "prompt")?;

        // Build trigger
        let trigger = match trigger_type {
            "cron" => {
                let schedule =
                    params
                        .get("schedule")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            ToolError::InvalidParameters(
                                "cron trigger requires 'schedule'".to_string(),
                            )
                        })?;
                let timezone = params
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .map(|tz| {
                        crate::timezone::parse_timezone(tz)
                            .map(|_| tz.to_string())
                            .ok_or_else(|| {
                                ToolError::InvalidParameters(format!(
                                    "invalid IANA timezone: '{tz}'"
                                ))
                            })
                    })
                    .transpose()?;
                // Validate cron expression
                next_cron_fire(schedule, timezone.as_deref()).map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid cron schedule: {e}"))
                })?;
                Trigger::Cron {
                    schedule: schedule.to_string(),
                    timezone,
                }
            }
            "event" => {
                let pattern = params
                    .get("event_pattern")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "event trigger requires 'event_pattern'".to_string(),
                        )
                    })?;
                // Validate regex
                regex::Regex::new(pattern)
                    .map_err(|e| ToolError::InvalidParameters(format!("invalid regex: {e}")))?;
                let channel = params
                    .get("event_channel")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                Trigger::Event {
                    channel,
                    pattern: pattern.to_string(),
                }
            }
            "system_event" => {
                let source = params
                    .get("event_source")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "system_event trigger requires 'event_source'".to_string(),
                        )
                    })?;
                let event_type = params
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(
                            "system_event trigger requires 'event_type'".to_string(),
                        )
                    })?;
                let filters = params
                    .get("event_filters")
                    .and_then(|v| v.as_object())
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| {
                                crate::agent::routine::json_value_as_filter_string(v)
                                    .map(|s| (k.to_string(), s))
                            })
                            .collect::<std::collections::HashMap<String, String>>()
                    })
                    .unwrap_or_default();
                Trigger::SystemEvent {
                    source: source.to_string(),
                    event_type: event_type.to_string(),
                    filters,
                }
            }
            "manual" => Trigger::Manual,
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown trigger_type: {other}"
                )));
            }
        };

        // Build action
        let action_type = params
            .get("action_type")
            .and_then(|v| v.as_str())
            .unwrap_or("lightweight");

        let context_paths: Vec<String> = params
            .get("context_paths")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let use_tools = params
            .get("use_tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_tool_rounds = params
            .get("max_tool_rounds")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, crate::agent::routine::MAX_TOOL_ROUNDS_LIMIT as u64) as u32)
            .unwrap_or(3);

        let action = match action_type {
            "lightweight" => RoutineAction::Lightweight {
                prompt: prompt.to_string(),
                context_paths,
                max_tokens: 4096,
                use_tools,
                max_tool_rounds,
            },
            "full_job" => {
                let tool_permissions = crate::agent::routine::parse_tool_permissions(&params);
                RoutineAction::FullJob {
                    title: name.to_string(),
                    description: prompt.to_string(),
                    max_iterations: 10,
                    tool_permissions,
                }
            }
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "unknown action_type: {other}"
                )));
            }
        };

        let cooldown_secs = params
            .get("cooldown_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        // Compute next fire time for cron
        let next_fire = if let Trigger::Cron {
            ref schedule,
            ref timezone,
        } = trigger
        {
            next_cron_fire(schedule, timezone.as_deref()).unwrap_or(None)
        } else {
            None
        };

        let routine = Routine {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: description.to_string(),
            user_id: ctx.user_id.clone(),
            enabled: true,
            trigger,
            action,
            guardrails: RoutineGuardrails {
                cooldown: Duration::from_secs(cooldown_secs),
                max_concurrent: 1,
                dedup_window: None,
            },
            notify: NotifyConfig {
                channel: params
                    .get("notify_channel")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                user: params
                    .get("notify_user")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                ..NotifyConfig::default()
            },
            last_run_at: None,
            next_fire_at: next_fire,
            run_count: 0,
            consecutive_failures: 0,
            state: serde_json::json!({}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.store
            .create_routine(&routine)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to create routine: {e}")))?;

        // Refresh event cache if this is an event trigger
        if matches!(
            routine.trigger,
            Trigger::Event { .. } | Trigger::SystemEvent { .. }
        ) {
            self.engine.refresh_event_cache().await;
        }

        let result = serde_json::json!({
            "id": routine.id.to_string(),
            "name": routine.name,
            "trigger_type": routine.trigger.type_tag(),
            "next_fire_at": routine.next_fire_at.map(|t| t.to_rfc3339()),
            "status": "created",
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_list ====================

pub struct RoutineListTool {
    store: Arc<dyn Database>,
}

impl RoutineListTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RoutineListTool {
    fn name(&self) -> &str {
        "routine_list"
    }

    fn description(&self) -> &str {
        "List all routines with their status, trigger info, and next fire time."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let routines = self
            .store
            .list_routines(&ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to list routines: {e}")))?;

        let list: Vec<serde_json::Value> = routines
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id.to_string(),
                    "name": r.name,
                    "description": r.description,
                    "enabled": r.enabled,
                    "trigger_type": r.trigger.type_tag(),
                    "action_type": r.action.type_tag(),
                    "last_run_at": r.last_run_at.map(|t| t.to_rfc3339()),
                    "next_fire_at": r.next_fire_at.map(|t| t.to_rfc3339()),
                    "run_count": r.run_count,
                    "consecutive_failures": r.consecutive_failures,
                })
            })
            .collect();

        let result = serde_json::json!({
            "count": list.len(),
            "routines": list,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_update ====================

pub struct RoutineUpdateTool {
    store: Arc<dyn Database>,
    engine: Arc<RoutineEngine>,
}

impl RoutineUpdateTool {
    pub fn new(store: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Self {
        Self { store, engine }
    }
}

#[async_trait]
impl Tool for RoutineUpdateTool {
    fn name(&self) -> &str {
        "routine_update"
    }

    fn description(&self) -> &str {
        "Update an existing routine. Can modify trigger, prompt, schedule, or toggle enabled state. \
         Pass the routine name and only the fields you want to change."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the routine to update"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Enable or disable the routine"
                },
                "prompt": {
                    "type": "string",
                    "description": "New prompt/instructions"
                },
                "schedule": {
                    "type": "string",
                    "description": "New cron schedule (for cron triggers)"
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone for cron schedule (e.g. 'America/New_York'). Only valid for cron triggers."
                },
                "description": {
                    "type": "string",
                    "description": "New description"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let mut routine = self
            .store
            .get_routine_by_name(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB error: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("routine '{}' not found", name)))?;

        // Apply updates
        if let Some(enabled) = params.get("enabled").and_then(|v| v.as_bool()) {
            routine.enabled = enabled;
        }

        if let Some(desc) = params.get("description").and_then(|v| v.as_str()) {
            routine.description = desc.to_string();
        }

        if let Some(prompt) = params.get("prompt").and_then(|v| v.as_str()) {
            match &mut routine.action {
                RoutineAction::Lightweight { prompt: p, .. } => *p = prompt.to_string(),
                RoutineAction::FullJob { description: d, .. } => *d = prompt.to_string(),
            }
        }

        // Validate timezone param if provided
        let new_timezone = params
            .get("timezone")
            .and_then(|v| v.as_str())
            .map(|tz| {
                crate::timezone::parse_timezone(tz)
                    .map(|_| tz.to_string())
                    .ok_or_else(|| {
                        ToolError::InvalidParameters(format!("invalid IANA timezone: '{tz}'"))
                    })
            })
            .transpose()?;

        let new_schedule = params.get("schedule").and_then(|v| v.as_str());

        if new_schedule.is_some() || new_timezone.is_some() {
            // Extract existing cron fields (cloned to avoid borrow conflict)
            let existing_cron = match &routine.trigger {
                Trigger::Cron { schedule, timezone } => Some((schedule.clone(), timezone.clone())),
                _ => None,
            };

            if let Some((old_schedule, old_tz)) = existing_cron {
                let effective_schedule = new_schedule.unwrap_or(&old_schedule);
                let effective_tz = new_timezone.or(old_tz);
                // Validate
                next_cron_fire(effective_schedule, effective_tz.as_deref()).map_err(|e| {
                    ToolError::InvalidParameters(format!("invalid cron schedule: {e}"))
                })?;

                routine.trigger = Trigger::Cron {
                    schedule: effective_schedule.to_string(),
                    timezone: effective_tz.clone(),
                };
                routine.next_fire_at =
                    next_cron_fire(effective_schedule, effective_tz.as_deref()).unwrap_or(None);
            } else {
                return Err(ToolError::InvalidParameters(
                    "Cannot update schedule or timezone on a non-cron routine.".to_string(),
                ));
            }
        }

        self.store
            .update_routine(&routine)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to update: {e}")))?;

        // Refresh event cache in case trigger changed
        self.engine.refresh_event_cache().await;

        let result = serde_json::json!({
            "name": routine.name,
            "enabled": routine.enabled,
            "trigger_type": routine.trigger.type_tag(),
            "next_fire_at": routine.next_fire_at.map(|t| t.to_rfc3339()),
            "status": "updated",
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_delete ====================

pub struct RoutineDeleteTool {
    store: Arc<dyn Database>,
    engine: Arc<RoutineEngine>,
}

impl RoutineDeleteTool {
    pub fn new(store: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Self {
        Self { store, engine }
    }
}

#[async_trait]
impl Tool for RoutineDeleteTool {
    fn name(&self) -> &str {
        "routine_delete"
    }

    fn description(&self) -> &str {
        "Delete a routine permanently. This also removes all run history."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the routine to delete"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let routine = self
            .store
            .get_routine_by_name(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB error: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("routine '{}' not found", name)))?;

        let deleted = self
            .store
            .delete_routine(routine.id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to delete: {e}")))?;

        // Refresh event cache
        self.engine.refresh_event_cache().await;

        let result = serde_json::json!({
            "name": name,
            "deleted": deleted,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_fire ====================

pub struct RoutineFireTool {
    store: Arc<dyn Database>,
    engine: Arc<RoutineEngine>,
}

impl RoutineFireTool {
    pub fn new(store: Arc<dyn Database>, engine: Arc<RoutineEngine>) -> Self {
        Self { store, engine }
    }
}

#[async_trait]
impl Tool for RoutineFireTool {
    fn name(&self) -> &str {
        "routine_fire"
    }

    fn description(&self) -> &str {
        "Manually trigger a routine to run immediately, bypassing schedule, trigger type, and cooldown."
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Firing a routine can dispatch a full_job with pre-authorized Always-gated tools,
        // so this is a meaningful escalation that warrants auto-approval gating.
        ApprovalRequirement::UnlessAutoApproved
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the routine to fire"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let routine = self
            .store
            .get_routine_by_name(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB error: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("routine '{}' not found", name)))?;

        let run_id = self
            .engine
            .fire_manual(routine.id, None)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("failed to fire routine '{}': {e}", name))
            })?;

        let result = serde_json::json!({
            "name": name,
            "run_id": run_id.to_string(),
            "status": "fired",
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== routine_history ====================

pub struct RoutineHistoryTool {
    store: Arc<dyn Database>,
}

impl RoutineHistoryTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RoutineHistoryTool {
    fn name(&self) -> &str {
        "routine_history"
    }

    fn description(&self) -> &str {
        "View the execution history of a routine. Shows recent runs with status, duration, and results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the routine"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max runs to return (default: 10)",
                    "default": 10
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(10)
            .min(50);

        let routine = self
            .store
            .get_routine_by_name(&ctx.user_id, name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("DB error: {e}")))?
            .ok_or_else(|| ToolError::ExecutionFailed(format!("routine '{}' not found", name)))?;

        let runs = self
            .store
            .list_routine_runs(routine.id, limit)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to list runs: {e}")))?;

        let run_list: Vec<serde_json::Value> = runs
            .iter()
            .map(|r| {
                let duration_secs = r
                    .completed_at
                    .map(|c| c.signed_duration_since(r.started_at).num_seconds());
                serde_json::json!({
                    "id": r.id.to_string(),
                    "trigger_type": r.trigger_type,
                    "trigger_detail": r.trigger_detail,
                    "started_at": r.started_at.to_rfc3339(),
                    "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
                    "duration_secs": duration_secs,
                    "status": r.status.to_string(),
                    "result_summary": r.result_summary,
                    "tokens_used": r.tokens_used,
                })
            })
            .collect();

        let result = serde_json::json!({
            "routine": name,
            "total_runs": routine.run_count,
            "runs": run_list,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== event_emit ====================

pub struct EventEmitTool {
    engine: Arc<RoutineEngine>,
}

impl EventEmitTool {
    pub fn new(engine: Arc<RoutineEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for EventEmitTool {
    fn name(&self) -> &str {
        "event_emit"
    }

    fn description(&self) -> &str {
        "Emit a structured system event to routines with a system_event trigger. \
         Use this to trigger routines from tool workflows without waiting for cron."
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Emitting an event can fire system_event routines that dispatch full_jobs
        // with pre-authorized Always-gated tools — same escalation risk as routine_fire.
        ApprovalRequirement::UnlessAutoApproved
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "event_source": {
                    "type": "string",
                    "description": "Event source (e.g. 'github', 'workflow', 'tool')"
                },
                "event_type": {
                    "type": "string",
                    "description": "Event type (e.g. 'issue.opened', 'pr.ready')"
                },
                "payload": {
                    "type": "object",
                    "description": "Structured event payload"
                }
            },
            "required": ["event_source", "event_type"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let source = require_str(&params, "event_source")?;
        let event_type = require_str(&params, "event_type")?;
        let payload = params
            .get("payload")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        let fired = self
            .engine
            .emit_system_event(source, event_type, &payload, Some(&ctx.user_id))
            .await;

        let result = serde_json::json!({
            "event_source": source,
            "event_type": event_type,
            "user_id": &ctx.user_id,
            "fired_routines": fired,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true
    }
}
