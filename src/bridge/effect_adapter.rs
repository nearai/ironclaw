//! Effect bridge adapter — wraps `ToolRegistry` + `SafetyLayer` as `ironclaw_engine::EffectExecutor`.
//!
//! This is the security boundary between the engine and existing IronClaw
//! infrastructure. All v1 security controls are enforced here:
//! - Tool approval (requires_approval, auto-approve tracking)
//! - Output sanitization (sanitize_tool_output + wrap_for_llm)
//! - Hook interception (BeforeToolCall)
//! - Sensitive parameter redaction
//! - Rate limiting (per-user, per-tool)

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;
use tracing::debug;

use ironclaw_engine::{
    ActionDef, ActionResult, CapabilityLease, EffectExecutor, EngineError, ThreadExecutionContext,
};

use crate::auth::oauth::sanitize_auth_url;
use crate::bridge::auth_manager::{AuthCheckResult, AuthManager};
use crate::bridge::router::synthetic_action_call_id;
use crate::context::JobContext;
use crate::hooks::{HookEvent, HookOutcome, HookRegistry};
use crate::tools::permissions::{PermissionState, effective_permission};
use crate::tools::rate_limiter::RateLimiter;
use crate::tools::{ApprovalRequirement, ToolRegistry};
use ironclaw_safety::SafetyLayer;

/// Wraps the existing tool pipeline to implement the engine's `EffectExecutor`.
///
/// Enforces all v1 security controls at the adapter boundary:
/// tool approval, output sanitization, hooks, rate limiting, and call limits.
pub struct EffectBridgeAdapter {
    tools: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
    hooks: Arc<HookRegistry>,
    /// Global auto-approve mode from agent config/env.
    auto_approve_tools: bool,
    /// Tools the user has approved with "always" (persists within session).
    auto_approved: RwLock<HashSet<String>>,
    /// Per-step tool call counter (reset externally between steps).
    call_count: std::sync::atomic::AtomicU32,
    /// Per-user per-tool sliding window rate limiter.
    rate_limiter: RateLimiter,
    /// Mission manager for handling mission_* function calls.
    mission_manager: RwLock<Option<Arc<ironclaw_engine::MissionManager>>>,
    /// Centralized auth manager for pre-flight credential checks.
    auth_manager: RwLock<Option<Arc<AuthManager>>>,
    /// Optional HTTP interceptor for trace recording / replay. When set, every
    /// tool call dispatched through this adapter gets it stamped onto its
    /// `JobContext`, so the built-in `http`/`web_fetch`/etc. tools route their
    /// outbound requests through the interceptor. Without this, engine v2 tool
    /// calls bypass the recorder entirely — recorded traces end up with zero
    /// `http_exchanges` and replay can't substitute responses.
    http_interceptor: RwLock<Option<Arc<dyn crate::llm::recording::HttpInterceptor>>>,
}

impl EffectBridgeAdapter {
    pub fn new(
        tools: Arc<ToolRegistry>,
        safety: Arc<SafetyLayer>,
        hooks: Arc<HookRegistry>,
    ) -> Self {
        Self {
            tools,
            safety,
            hooks,
            auto_approve_tools: false,
            auto_approved: RwLock::new(HashSet::new()),
            call_count: std::sync::atomic::AtomicU32::new(0),
            rate_limiter: RateLimiter::new(),
            mission_manager: RwLock::new(None),
            auth_manager: RwLock::new(None),
            http_interceptor: RwLock::new(None),
        }
    }

    /// Install the trace HTTP interceptor on this adapter. Every JobContext
    /// the adapter constructs for tool dispatch will carry a clone of this
    /// interceptor, so http-aware tools will record/replay through it.
    pub async fn set_http_interceptor(
        &self,
        interceptor: Arc<dyn crate::llm::recording::HttpInterceptor>,
    ) {
        *self.http_interceptor.write().await = Some(interceptor);
    }

    /// Mirror the v1 dispatcher behavior for globally auto-approved tools.
    pub fn with_global_auto_approve(mut self, enabled: bool) -> Self {
        self.auto_approve_tools = enabled;
        self
    }

    /// Mark a tool as auto-approved (user said "always").
    pub async fn auto_approve_tool(&self, tool_name: &str) {
        self.auto_approved
            .write()
            .await
            .insert(tool_name.to_string());
    }

    /// Revoke auto-approve for a tool (rollback on resume failure).
    pub async fn revoke_auto_approve(&self, tool_name: &str) {
        self.auto_approved.write().await.remove(tool_name);
    }

    /// Access the underlying tool registry (for param redaction, etc.).
    pub fn tools(&self) -> &Arc<ToolRegistry> {
        &self.tools
    }

    /// Set the auth manager for pre-flight credential checks.
    pub async fn set_auth_manager(&self, mgr: Arc<AuthManager>) {
        *self.auth_manager.write().await = Some(mgr);
    }

    /// Set the mission manager (called after engine init).
    pub async fn set_mission_manager(&self, mgr: Arc<ironclaw_engine::MissionManager>) {
        *self.mission_manager.write().await = Some(mgr);
    }

    /// Get the mission manager if available.
    pub async fn mission_manager(&self) -> Option<Arc<ironclaw_engine::MissionManager>> {
        self.mission_manager.read().await.clone()
    }

    fn gate_paused(
        gate_name: &str,
        action_name: &str,
        call_id: Option<&str>,
        parameters: serde_json::Value,
        resume_kind: ironclaw_engine::ResumeKind,
        resume_output: Option<serde_json::Value>,
    ) -> EngineError {
        EngineError::GatePaused {
            gate_name: gate_name.to_string(),
            action_name: action_name.to_string(),
            call_id: call_id.unwrap_or_default().to_string(),
            parameters: Box::new(parameters),
            resume_kind: Box::new(resume_kind),
            resume_output: resume_output.map(Box::new),
        }
    }

    fn auth_gate_from_extension_result(
        action_name: &str,
        parameters: serde_json::Value,
        context: &ThreadExecutionContext,
        output_value: &serde_json::Value,
    ) -> Option<EngineError> {
        let status = output_value.get("status").and_then(|v| v.as_str())?;
        let name = output_value.get("name").and_then(|v| v.as_str())?;

        match status {
            "awaiting_authorization" | "awaiting_token" => Some(Self::gate_paused(
                "authentication",
                action_name,
                context.current_call_id.as_deref(),
                parameters,
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name: output_value
                        .get("credential_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(name)
                        .to_string(),
                    instructions: output_value
                        .get("instructions")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Complete authentication to continue.")
                        .to_string(),
                    auth_url: sanitize_auth_url(
                        output_value.get("auth_url").and_then(|v| v.as_str()),
                    ),
                },
                None,
            )),
            _ => None,
        }
    }

    /// Handle mission_* and routine_* function calls. routine_* are aliases:
    /// the routine schema is translated into mission_* parameters and
    /// dispatched through the same mission manager. Returns None if the
    /// action name is neither a mission nor routine call.
    async fn handle_mission_call(
        &self,
        action_name: &str,
        params: &serde_json::Value,
        context: &ThreadExecutionContext,
    ) -> Option<Result<ActionResult, EngineError>> {
        // Translate routine_* aliases to mission_* before dispatching. The
        // routine schema is richer (kind/schedule/pattern/source/event_type/
        // filters/execution/delivery/advanced) than mission_*; the translator
        // collapses it into mission fields plus a follow-up update for the
        // non-execution guardrails (cooldown, max_concurrent, dedup_window,
        // notify_user, context_paths, description).
        let routine_alias = routine_to_mission_alias(action_name, params);
        let (effective_action, effective_params, post_create_update) =
            if let Some(alias) = routine_alias.as_ref() {
                (
                    alias.mission_action,
                    std::borrow::Cow::Borrowed(&alias.mission_params),
                    alias.post_create_update.clone(),
                )
            } else {
                (action_name, std::borrow::Cow::Borrowed(params), None)
            };
        let action_name = effective_action;
        let params = effective_params.as_ref();

        let mgr = self.mission_manager.read().await;
        let mgr = mgr.as_ref()?;

        let result = match action_name {
            "mission_create" => {
                let name = params
                    .get("name")
                    .or_else(|| params.get("_args").and_then(|a| a.get(0)))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unnamed mission");
                let goal = params
                    .get("goal")
                    .or_else(|| params.get("_args").and_then(|a| a.get(1)))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let cadence_str = params
                    .get("cadence")
                    .or_else(|| params.get("_args").and_then(|a| a.get(2)))
                    .and_then(|v| v.as_str());
                let Some(cadence_str) = cadence_str else {
                    return Some(Ok(ActionResult {
                        call_id: context
                            .current_call_id
                            .clone()
                            .unwrap_or_else(|| synthetic_action_call_id(action_name)),
                        action_name: action_name.to_string(),
                        output: serde_json::json!({
                            "error": concat!(
                                "cadence is required. Use 'manual', a cron expression ",
                                "(e.g. '0 9 * * *'), 'event:<channel>:<pattern>' ",
                                "(e.g. 'event:telegram:.*'), or 'webhook:<path>'"
                            )
                        }),
                        is_error: true,
                        duration: std::time::Duration::ZERO,
                    }));
                };
                // Use explicit timezone param, fall back to user's channel timezone.
                // ValidTimezone::parse filters empty/invalid strings.
                let timezone = params
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .and_then(ironclaw_engine::ValidTimezone::parse)
                    .or(context.user_timezone);
                let cadence = match parse_cadence(cadence_str, timezone) {
                    Ok(c) => c,
                    Err(msg) => {
                        return Some(Ok(ActionResult {
                            call_id: context
                                .current_call_id
                                .clone()
                                .unwrap_or_else(|| synthetic_action_call_id(action_name)),
                            action_name: action_name.to_string(),
                            output: serde_json::json!({"error": msg}),
                            is_error: true,
                            duration: std::time::Duration::ZERO,
                        }));
                    }
                };
                // notify_channels: explicit array, or default to current channel
                let notify_channels =
                    if let Some(arr) = params.get("notify_channels").and_then(|v| v.as_array()) {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    } else if let Some(ch) = &context.source_channel {
                        vec![ch.clone()]
                    } else {
                        vec![]
                    };
                // Validate guardrail params before creating the mission so
                // a type mismatch doesn't leave a "ghost" mission in storage.
                let mut guardrail_updates = post_create_update.clone().unwrap_or_default();
                if let Err(msg) = extract_guardrails(params, &mut guardrail_updates) {
                    return Some(Ok(ActionResult {
                        call_id: context
                            .current_call_id
                            .clone()
                            .unwrap_or_else(|| synthetic_action_call_id(action_name)),
                        action_name: action_name.to_string(),
                        output: serde_json::json!({"error": msg}),
                        is_error: true,
                        duration: std::time::Duration::ZERO,
                    }));
                }
                if let Some(criteria) = params.get("success_criteria").and_then(|v| v.as_str()) {
                    guardrail_updates.success_criteria = Some(criteria.to_string());
                }
                match mgr
                    .create_mission(
                        context.project_id,
                        &context.user_id,
                        name,
                        goal,
                        cadence,
                        notify_channels,
                    )
                    .await
                {
                    Ok(id) => {
                        let has_updates = guardrail_updates.cooldown_secs.is_some()
                            || guardrail_updates.max_concurrent.is_some()
                            || guardrail_updates.dedup_window_secs.is_some()
                            || guardrail_updates.max_threads_per_day.is_some()
                            || guardrail_updates.description.is_some()
                            || guardrail_updates.context_paths.is_some()
                            || guardrail_updates.notify_user.is_some()
                            || guardrail_updates.notify_channels.is_some()
                            || guardrail_updates.cadence.is_some()
                            || guardrail_updates.success_criteria.is_some();
                        let mut warnings: Vec<String> = Vec::new();
                        if has_updates
                            && let Err(e) = mgr
                                .update_mission(id, &context.user_id, guardrail_updates)
                                .await
                        {
                            tracing::warn!(
                                mission_id = %id,
                                error = %e,
                                "routine alias: failed to apply post-create updates"
                            );
                            warnings.push(format!(
                                "post-create update failed: {e}. The mission was created but \
                                 the cadence/context_paths/cooldown/notify fields from the \
                                 routine schema were NOT applied. Call update_mission to retry."
                            ));
                        }
                        if warnings.is_empty() {
                            Ok(serde_json::json!({
                                "mission_id": id.to_string(),
                                "name": name,
                                "status": "created"
                            }))
                        } else {
                            Ok(serde_json::json!({
                                "mission_id": id.to_string(),
                                "name": name,
                                "status": "created_with_warnings",
                                "warnings": warnings
                            }))
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            "mission_list" => match mgr
                .list_missions(context.project_id, &context.user_id)
                .await
            {
                Ok(missions) => {
                    let list: Vec<serde_json::Value> = missions
                        .iter()
                        .map(|m| {
                            let timezone =
                                if let ironclaw_engine::types::mission::MissionCadence::Cron {
                                    timezone: Some(tz),
                                    ..
                                } = &m.cadence
                                {
                                    serde_json::Value::String(tz.to_string())
                                } else {
                                    serde_json::Value::Null
                                };
                            serde_json::json!({
                                "id": m.id.to_string(),
                                "name": m.name,
                                "goal": m.goal,
                                "status": format!("{:?}", m.status),
                                "cadence": cadence_to_round_trip_string(&m.cadence),
                                "timezone": timezone,
                                "threads": m.thread_history.len(),
                                "current_focus": m.current_focus,
                                "notify_channels": m.notify_channels,
                                "cooldown_secs": m.cooldown_secs,
                                "max_concurrent": m.max_concurrent,
                                "dedup_window_secs": m.dedup_window_secs,
                                "max_threads_per_day": m.max_threads_per_day,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(list))
                }
                Err(e) => Err(e),
            },
            "mission_fire" => {
                let id_str = params
                    .get("id")
                    .or_else(|| params.get("_args").and_then(|a| a.get(0)))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let id = uuid::Uuid::parse_str(id_str)
                    .map(ironclaw_engine::MissionId)
                    .map_err(|e| EngineError::Effect {
                        reason: format!("invalid mission id: {e}"),
                    });
                match id {
                    Ok(id) => match mgr.fire_mission(id, &context.user_id, None).await {
                        Ok(Some(tid)) => {
                            Ok(serde_json::json!({"thread_id": tid.to_string(), "status": "fired"}))
                        }
                        Ok(None) => Ok(
                            serde_json::json!({"status": "not_fired", "reason": "mission is terminal or budget exhausted"}),
                        ),
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                }
            }
            "mission_pause" | "mission_resume" => {
                let id_str = params
                    .get("id")
                    .or_else(|| params.get("_args").and_then(|a| a.get(0)))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let id = uuid::Uuid::parse_str(id_str)
                    .map(ironclaw_engine::MissionId)
                    .map_err(|e| EngineError::Effect {
                        reason: format!("invalid mission id: {e}"),
                    });
                match id {
                    Ok(id) => {
                        let res = if action_name == "mission_pause" {
                            mgr.pause_mission(id, &context.user_id).await
                        } else {
                            mgr.resume_mission(id, &context.user_id).await
                        };
                        match res {
                            Ok(()) => Ok(serde_json::json!({"status": "ok"})),
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            "mission_complete" => {
                let id_str = params
                    .get("id")
                    .or_else(|| params.get("name")) // routine_delete uses "name" param
                    .or_else(|| params.get("_args").and_then(|a| a.get(0)))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let id = uuid::Uuid::parse_str(id_str)
                    .map(ironclaw_engine::MissionId)
                    .map_err(|e| EngineError::Effect {
                        reason: format!("invalid mission id: {e}"),
                    });
                match id {
                    Ok(id) => match mgr.complete_mission(id).await {
                        Ok(()) => Ok(serde_json::json!({"status": "completed"})),
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                }
            }
            "mission_update" => {
                let id_str = params
                    .get("id")
                    .or_else(|| params.get("_args").and_then(|a| a.get(0)))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let id = uuid::Uuid::parse_str(id_str)
                    .map(ironclaw_engine::MissionId)
                    .map_err(|e| EngineError::Effect {
                        reason: format!("invalid mission id: {e}"),
                    });
                match id {
                    Ok(id) => {
                        let mut updates = ironclaw_engine::MissionUpdate::default();
                        if let Some(name) = params.get("name").and_then(|v| v.as_str()) {
                            updates.name = Some(name.to_string());
                        }
                        if let Some(goal) = params.get("goal").and_then(|v| v.as_str()) {
                            updates.goal = Some(goal.to_string());
                        }
                        if let Some(cadence) = params.get("cadence").and_then(|v| v.as_str()) {
                            let tz = params
                                .get("timezone")
                                .and_then(|v| v.as_str())
                                .and_then(ironclaw_engine::ValidTimezone::parse)
                                .or(context.user_timezone);
                            match parse_cadence(cadence, tz) {
                                Ok(c) => updates.cadence = Some(c),
                                Err(msg) => {
                                    return Some(Ok(ActionResult {
                                        call_id: context.current_call_id.clone().unwrap_or_else(
                                            || synthetic_action_call_id(action_name),
                                        ),
                                        action_name: action_name.to_string(),
                                        output: serde_json::json!({"error": msg}),
                                        is_error: true,
                                        duration: std::time::Duration::ZERO,
                                    }));
                                }
                            }
                        }
                        if let Some(arr) = params.get("notify_channels").and_then(|v| v.as_array())
                        {
                            updates.notify_channels = Some(
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect(),
                            );
                        }
                        if let Err(msg) = extract_guardrails(params, &mut updates) {
                            return Some(Ok(ActionResult {
                                call_id: context
                                    .current_call_id
                                    .clone()
                                    .unwrap_or_else(|| synthetic_action_call_id(action_name)),
                                action_name: action_name.to_string(),
                                output: serde_json::json!({"error": msg}),
                                is_error: true,
                                duration: std::time::Duration::ZERO,
                            }));
                        }
                        if let Some(criteria) =
                            params.get("success_criteria").and_then(|v| v.as_str())
                        {
                            updates.success_criteria = Some(criteria.to_string());
                        }
                        match mgr.update_mission(id, &context.user_id, updates).await {
                            Ok(()) => Ok(serde_json::json!({"status": "updated"})),
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            _ => return None, // Not a mission/routine call
        };

        // Use the live call_id from the executing thread context, falling
        // back to a synthetic id when none is available. An empty `call_id`
        // on an `ActionResult` corrupts the engine's call/result pairing
        // and causes the assistant to drop the response (see the doc on
        // `crate::bridge::router::resolved_call_id_for_pending_action`).
        let call_id = context
            .current_call_id
            .clone()
            .unwrap_or_else(|| synthetic_action_call_id(action_name));

        Some(match result {
            Ok(output) => Ok(ActionResult {
                call_id: call_id.clone(),
                action_name: action_name.to_string(),
                output,
                is_error: false,
                duration: std::time::Duration::ZERO,
            }),
            Err(e) => Ok(ActionResult {
                call_id,
                action_name: action_name.to_string(),
                output: serde_json::json!({"error": e.to_string()}),
                is_error: true,
                duration: std::time::Duration::ZERO,
            }),
        })
    }

    /// Reset the per-step call counter (called between threads/steps).
    pub fn reset_call_count(&self) {
        self.call_count
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    pub async fn execute_resolved_pending_action(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        lease: &CapabilityLease,
        context: &ThreadExecutionContext,
        approval_already_granted: bool,
    ) -> Result<ActionResult, EngineError> {
        self.execute_action_internal(
            action_name,
            parameters,
            lease,
            context,
            approval_already_granted,
        )
        .await
    }

    async fn execute_action_internal(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        _lease: &CapabilityLease,
        context: &ThreadExecutionContext,
        approval_already_granted: bool,
    ) -> Result<ActionResult, EngineError> {
        let start = Instant::now();

        let resolved_name = self.tools.resolve_name(action_name).await;
        let mut lookup_name = resolved_name.as_deref().unwrap_or(action_name).to_string();

        // ── Per-step call limit (prevent amplification loops) ──
        const MAX_CALLS_PER_STEP: u32 = 50;
        let count = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if count >= MAX_CALLS_PER_STEP {
            return Err(EngineError::Effect {
                reason: format!(
                    "Tool call limit reached ({MAX_CALLS_PER_STEP} per code step). \
                     Break your task into multiple steps."
                ),
            });
        }

        if let Some(result) = self
            .handle_mission_call(action_name, &parameters, context)
            .await
        {
            return result.map(|mut r| {
                r.duration = start.elapsed();
                r
            });
        }

        if is_v1_only_tool(&lookup_name) {
            return Err(EngineError::Effect {
                reason: format!(
                    "Tool '{}' is not available in engine v2. \
                     Tell the user to use the slash command instead (e.g. /routine, /job).",
                    action_name
                ),
            });
        }

        if is_v1_auth_tool(&lookup_name) {
            return Err(EngineError::Effect {
                reason: format!(
                    "Tool '{}' is not available in engine v2. \
                     Authentication is handled automatically by the kernel.",
                    action_name
                ),
            });
        }

        if resolved_name.is_none()
            && let Some(auth_mgr) = self.auth_manager.read().await.as_ref()
            && let Some(latent_execution) = auth_mgr
                .execute_latent_extension_action(action_name, &context.user_id)
                .await
        {
            match latent_execution {
                Ok(crate::bridge::auth_manager::LatentActionExecution::RetryRegisteredAction {
                    resolved_action,
                }) => {
                    lookup_name = resolved_action;
                }
                Ok(crate::bridge::auth_manager::LatentActionExecution::ProviderReady {
                    provider_extension,
                    available_actions,
                }) => {
                    return Ok(ActionResult {
                        call_id: context
                            .current_call_id
                            .clone()
                            .unwrap_or_else(|| synthetic_action_call_id(action_name)),
                        action_name: action_name.to_string(),
                        output: serde_json::json!({
                            "provider_extension": provider_extension,
                            "available_actions": available_actions,
                            "message": "Provider is ready. Use one of the available provider actions next."
                        }),
                        is_error: false,
                        duration: start.elapsed(),
                    });
                }
                Ok(crate::bridge::auth_manager::LatentActionExecution::NeedsAuth {
                    credential_name,
                    instructions,
                    auth_url,
                }) => {
                    return Err(Self::gate_paused(
                        "authentication",
                        action_name,
                        context.current_call_id.as_deref(),
                        parameters,
                        ironclaw_engine::ResumeKind::Authentication {
                            credential_name,
                            instructions,
                            auth_url: sanitize_auth_url(auth_url.as_deref()),
                        },
                        None,
                    ));
                }
                Ok(crate::bridge::auth_manager::LatentActionExecution::NeedsSetup { message }) => {
                    return Err(EngineError::Effect { reason: message });
                }
                Err(err) => {
                    return Err(EngineError::Effect {
                        reason: err.to_string(),
                    });
                }
            }
        }

        if let Some(tool) = self.tools.get(&lookup_name).await
            && let Some(rl_config) = tool.rate_limit_config()
        {
            let result = self
                .rate_limiter
                .check_and_record(&context.user_id, &lookup_name, &rl_config)
                .await;
            if let crate::tools::rate_limiter::RateLimitResult::Limited { retry_after, .. } = result
            {
                return Err(EngineError::Effect {
                    reason: format!(
                        "Tool '{}' is rate limited. Try again in {:.0}s.",
                        action_name,
                        retry_after.as_secs_f64()
                    ),
                });
            }
        }

        {
            let has_mgr = self.auth_manager.read().await.is_some();
            let has_reg = self.tools.credential_registry().is_some();
            if !has_mgr || !has_reg {
                tracing::warn!(
                    tool = %lookup_name,
                    has_auth_manager = has_mgr,
                    has_credential_registry = has_reg,
                    "Pre-flight auth gate SKIPPED — missing dependency"
                );
            }
        }
        if let Some(auth_mgr) = self.auth_manager.read().await.as_ref()
            && let Some(registry) = self.tools.credential_registry()
        {
            match auth_mgr
                .check_action_auth(&lookup_name, &parameters, &context.user_id, registry)
                .await
            {
                AuthCheckResult::MissingCredentials(missing) => {
                    let cred = &missing[0];
                    debug!(
                        credential = %cred.credential_name,
                        tool = %lookup_name,
                        user = %context.user_id,
                        "Pre-flight auth: credential missing — blocking tool call"
                    );
                    return Err(Self::gate_paused(
                        "authentication",
                        action_name,
                        context.current_call_id.as_deref(),
                        parameters,
                        ironclaw_engine::ResumeKind::Authentication {
                            credential_name: cred.credential_name.clone(),
                            instructions: cred.setup_instructions.clone().unwrap_or_else(|| {
                                format!("Provide your {} token", cred.credential_name)
                            }),
                            auth_url: sanitize_auth_url(cred.auth_url.as_deref()),
                        },
                        None,
                    ));
                }
                AuthCheckResult::Ready => {
                    debug!(tool = %lookup_name, "Pre-flight auth: credentials present");
                }
                AuthCheckResult::NoAuthRequired => {}
            }
        }

        if let Some(provider_extension) = self.tools.provider_extension_for_tool(&lookup_name).await
            && let Some(auth_mgr) = self.auth_manager.read().await.as_ref()
        {
            use crate::bridge::auth_manager::ToolReadiness;
            match auth_mgr
                .check_tool_readiness(&provider_extension, &context.user_id)
                .await
            {
                ToolReadiness::NeedsAuth {
                    credential_name,
                    instructions,
                    auth_url,
                } => {
                    debug!(
                        provider_extension = %provider_extension,
                        action = %lookup_name,
                        credential = %credential_name,
                        "Pre-flight extension readiness: authentication required"
                    );
                    return Err(Self::gate_paused(
                        "authentication",
                        action_name,
                        context.current_call_id.as_deref(),
                        parameters,
                        ironclaw_engine::ResumeKind::Authentication {
                            credential_name,
                            instructions: instructions.unwrap_or_else(|| {
                                format!("Authenticate '{}' to continue.", provider_extension)
                            }),
                            auth_url: sanitize_auth_url(auth_url.as_deref()),
                        },
                        None,
                    ));
                }
                ToolReadiness::NeedsSetup { message } => {
                    return Err(EngineError::Effect {
                        reason: format!(
                            "Extension '{}' is not ready: {}",
                            provider_extension, message
                        ),
                    });
                }
                ToolReadiness::Ready => {}
            }
        }

        if let Some((_, tool)) = self.tools.get_resolved(&lookup_name).await {
            let user_permission = if let Some(db) = self.tools.database() {
                match db.get_all_settings(&context.user_id).await {
                    Ok(db_map) => {
                        let settings = crate::settings::Settings::from_db_map(&db_map);
                        Some(effective_permission(
                            &lookup_name,
                            &settings.tool_permissions,
                        ))
                    }
                    Err(error) => {
                        tracing::warn!(
                            user_id = %context.user_id,
                            tool = %lookup_name,
                            error = %error,
                            "Failed to load tool permission overrides for engine v2"
                        );
                        None
                    }
                }
            } else {
                None
            };

            if matches!(user_permission, Some(PermissionState::Disabled)) {
                return Err(EngineError::LeaseDenied {
                    reason: format!("Tool '{}' is disabled for this user.", action_name),
                });
            }

            let requirement = tool.requires_approval(&parameters);
            match requirement {
                ApprovalRequirement::Always => {
                    return Err(EngineError::LeaseDenied {
                        reason: format!(
                            "Tool '{}' requires explicit approval for this operation. \
                             This action cannot be auto-approved.",
                            action_name
                        ),
                    });
                }
                ApprovalRequirement::UnlessAutoApproved => {
                    let is_approved = self.auto_approve_tools
                        || self.auto_approved.read().await.contains(&lookup_name)
                        || matches!(user_permission, Some(PermissionState::AlwaysAllow));
                    if !is_approved && !approval_already_granted {
                        return Err(Self::gate_paused(
                            "approval",
                            action_name,
                            context.current_call_id.as_deref(),
                            parameters,
                            ironclaw_engine::ResumeKind::Approval { allow_always: true },
                            None,
                        ));
                    }
                }
                ApprovalRequirement::Never => {}
            }
        }

        let redacted_params = if let Some(tool) = self.tools.get(&lookup_name).await {
            crate::tools::redact_params(&parameters, tool.sensitive_params())
        } else {
            parameters.clone()
        };

        let hook_event = HookEvent::ToolCall {
            tool_name: lookup_name.to_string(),
            parameters: redacted_params,
            user_id: context.user_id.clone(),
            context: format!("engine_v2:{}", context.thread_id),
        };

        match self.hooks.run(&hook_event).await {
            Ok(HookOutcome::Reject { reason }) => {
                return Err(EngineError::LeaseDenied {
                    reason: format!("Tool '{}' blocked by hook: {}", action_name, reason),
                });
            }
            Err(crate::hooks::HookError::Rejected { reason }) => {
                return Err(EngineError::LeaseDenied {
                    reason: format!("Tool '{}' blocked by hook: {}", action_name, reason),
                });
            }
            Err(e) => {
                debug!(tool = lookup_name, error = %e, "hook error (fail-open)");
            }
            Ok(HookOutcome::Continue { .. }) => {}
        }

        let mut job_ctx = JobContext::with_user(
            &context.user_id,
            "engine_v2",
            format!("Thread {}", context.thread_id),
        );
        // Stamp the trace HTTP interceptor onto the per-call JobContext so
        // tools that respect it (http, web_fetch, etc.) route their outbound
        // requests through the recorder/replayer.
        if let Some(ref interceptor) = *self.http_interceptor.read().await {
            job_ctx.http_interceptor = Some(Arc::clone(interceptor));
        }

        let result = crate::tools::execute::execute_tool_with_safety(
            &self.tools,
            &self.safety,
            &lookup_name,
            parameters.clone(),
            &job_ctx,
        )
        .await;

        let duration = start.elapsed();

        match result {
            Ok(output) => {
                let sanitized = self.safety.sanitize_tool_output(&lookup_name, &output);
                let wrapped = self.safety.wrap_for_llm(&lookup_name, &sanitized.content);
                let output_value = serde_json::from_str::<serde_json::Value>(&output)
                    .unwrap_or(serde_json::Value::String(wrapped));

                if (lookup_name == "tool_activate"
                    || lookup_name == "tool_auth"
                    || lookup_name == "tool_install"
                    || lookup_name == "tool-install")
                    && let Some(err) = Self::auth_gate_from_extension_result(
                        action_name,
                        parameters.clone(),
                        context,
                        &output_value,
                    )
                {
                    return Err(err);
                }

                Ok(ActionResult {
                    call_id: context
                        .current_call_id
                        .clone()
                        .unwrap_or_else(|| synthetic_action_call_id(action_name)),
                    action_name: action_name.to_string(),
                    output: output_value,
                    is_error: false,
                    duration,
                })
            }
            Err(e) => {
                let error_msg = format!("Tool '{}' failed: {}", lookup_name, e);
                if error_msg.contains("authentication_required")
                    && let Some(cred_name) = extract_credential_name(&error_msg)
                    && self.is_known_credential(&cred_name)
                {
                    tracing::warn!(
                        credential = %cred_name,
                        tool = %lookup_name,
                        user = %context.user_id,
                        "Credential missing — returning GatePaused(authentication)"
                    );
                    return Err(Self::gate_paused(
                        "authentication",
                        action_name,
                        context.current_call_id.as_deref(),
                        parameters,
                        ironclaw_engine::ResumeKind::Authentication {
                            credential_name: cred_name.clone(),
                            instructions: format!("Provide your {} token", cred_name),
                            auth_url: None,
                        },
                        None,
                    ));
                }

                let sanitized = self.safety.sanitize_tool_output(&lookup_name, &error_msg);

                Ok(ActionResult {
                    call_id: context
                        .current_call_id
                        .clone()
                        .unwrap_or_else(|| synthetic_action_call_id(action_name)),
                    action_name: action_name.to_string(),
                    output: serde_json::json!({"error": sanitized.content}),
                    is_error: true,
                    duration,
                })
            }
        }
    }

    /// Defense against credential-name injection: a tool can fabricate an
    /// `authentication_required` error containing an attacker-chosen
    /// `credential_name` to phish the user. We only honor the gate request
    /// when the name corresponds to a credential the host has actually
    /// registered.
    ///
    /// **Fail-closed:** when no credential registry is wired, we reject the
    /// gate request rather than honoring it. A test/embed harness without a
    /// registry has no source of truth for credential names, and trusting
    /// the tool's claim in that mode would let any tool prompt the user for
    /// any credential name.
    fn is_known_credential(&self, credential_name: &str) -> bool {
        match self.tools.credential_registry() {
            Some(registry) => registry.has_secret(credential_name),
            None => false,
        }
    }
}

#[async_trait::async_trait]
impl EffectExecutor for EffectBridgeAdapter {
    async fn execute_action(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        lease: &CapabilityLease,
        context: &ThreadExecutionContext,
    ) -> Result<ActionResult, EngineError> {
        self.execute_action_internal(action_name, parameters, lease, context, false)
            .await
    }

    async fn available_actions(
        &self,
        _leases: &[CapabilityLease],
    ) -> Result<Vec<ActionDef>, EngineError> {
        let tool_defs = self.tools.tool_definitions().await;

        // Build action defs, excluding v1-only tools and v1 auth tools
        let mut actions = Vec::with_capacity(tool_defs.len());
        for td in tool_defs {
            // Skip tools that can't work in engine v2
            if is_v1_only_tool(&td.name) {
                continue;
            }

            // Skip v1 auth management tools — auth is kernel-level in v2
            if is_v1_auth_tool(&td.name) {
                continue;
            }

            let python_name = td.name.replace('-', "_");

            actions.push(ActionDef {
                name: python_name,
                description: td.description,
                parameters_schema: td.parameters,
                effects: vec![],
                // Approval is enforced at execute-time inside this adapter so
                // thread-scoped one-shot approvals and auth-aware bypasses can
                // participate. Advertising approval here would cause the engine
                // policy preflight to interrupt before the adapter can apply
                // those runtime checks.
                requires_approval: false,
            });
        }

        if let Some(auth_mgr) = self.auth_manager.read().await.as_ref() {
            for latent in auth_mgr.latent_extension_actions().await {
                if actions
                    .iter()
                    .any(|action| action.name == latent.action_name)
                {
                    continue;
                }
                actions.push(ActionDef {
                    name: latent.action_name,
                    description: latent.description,
                    parameters_schema: latent.parameters_schema,
                    effects: vec![],
                    requires_approval: false,
                });
            }
        }

        actions.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(actions)
    }
}

/// Strictly extract a u64 from a JSON value, rejecting wrong types.
fn strict_u64(params: &serde_json::Value, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("'{key}' must be an integer, got {v}")),
    }
}

/// Extract guardrail overrides from params, failing on type mismatches.
fn extract_guardrails(
    params: &serde_json::Value,
    base: &mut ironclaw_engine::MissionUpdate,
) -> Result<(), String> {
    if let Some(v) = strict_u64(params, "cooldown_secs")? {
        base.cooldown_secs = Some(v);
    }
    if let Some(v) = strict_u64(params, "max_concurrent")? {
        base.max_concurrent = Some(
            u32::try_from(v).map_err(|_| format!("'max_concurrent' value {v} exceeds u32 max"))?,
        );
    }
    if let Some(v) = strict_u64(params, "dedup_window_secs")? {
        base.dedup_window_secs = Some(v);
    }
    if let Some(v) = strict_u64(params, "max_threads_per_day")? {
        base.max_threads_per_day = Some(
            u32::try_from(v)
                .map_err(|_| format!("'max_threads_per_day' value {v} exceeds u32 max"))?,
        );
    }
    Ok(())
}

/// Parse a cadence string into a MissionCadence.
///
/// When cadence is a cron expression, `timezone` is used as the scheduling
/// timezone. This is typically the user's channel timezone, auto-injected
/// from `ThreadExecutionContext::user_timezone`.
///
/// Returns an error for unrecognized cadence strings so the LLM can correct
/// the call instead of silently falling back to Manual.
fn parse_cadence(
    s: &str,
    timezone: Option<ironclaw_engine::ValidTimezone>,
) -> Result<ironclaw_engine::types::mission::MissionCadence, String> {
    use ironclaw_engine::types::mission::MissionCadence;
    let trimmed = s.trim();
    let lower = trimmed.to_lowercase();
    // Check explicit prefixes BEFORE the cron heuristic. Otherwise an input
    // like `event: a b c d e` matches `split_whitespace().count() >= 5` and
    // is silently misclassified as a cron expression — the user said
    // "event:..." and gets a Cron cadence with a parse error downstream.
    if lower == "manual" {
        Ok(MissionCadence::Manual)
    } else if lower.starts_with("event:") {
        // Extract from original (not lowercased) to preserve case in regex patterns.
        let rest = trimmed["event:".len()..].trim();
        // Expected format: event:<channel>:<pattern>
        // Split on first ':' after the channel name.
        let (channel, pattern) = match rest.split_once(':') {
            Some((ch, pat)) if !ch.trim().is_empty() && !pat.trim().is_empty() => {
                (ch.trim(), pat.trim())
            }
            _ => {
                return Err(concat!(
                    "event cadence requires 'event:<channel>:<pattern>', ",
                    "e.g. 'event:telegram:.*' to match all messages on the telegram channel"
                )
                .to_string());
            }
        };
        // Validate with the same size limit the engine uses at runtime.
        if let Err(e) = regex::RegexBuilder::new(pattern)
            .size_limit(ironclaw_engine::runtime::mission::MAX_EVENT_REGEX_SIZE)
            .build()
        {
            return Err(format!(
                "event pattern '{pattern}' is not a valid regex: {e}"
            ));
        }
        Ok(MissionCadence::OnEvent {
            event_pattern: pattern.to_string(),
            channel: if channel == "*" {
                None
            } else {
                Some(channel.to_string())
            },
        })
    } else if lower.starts_with("system_event:") {
        // Round-trip format emitted by cadence_to_round_trip_string():
        //   system_event:<source>/<event_type>
        let rest = trimmed["system_event:".len()..].trim();
        let (source, event_type) = match rest.split_once('/') {
            Some((s, e)) if !s.trim().is_empty() && !e.trim().is_empty() => {
                (s.trim().to_string(), e.trim().to_string())
            }
            _ => {
                return Err(
                    "system_event cadence requires 'system_event:<source>/<event_type>', \
                     e.g. 'system_event:self-improvement/thread_completed'"
                        .to_string(),
                );
            }
        };
        Ok(MissionCadence::OnSystemEvent {
            source,
            event_type,
            filters: std::collections::HashMap::new(),
        })
    } else if lower.starts_with("webhook:") {
        // Extract from original to preserve case in webhook paths.
        let path = trimmed["webhook:".len()..].trim().to_string();
        if path.is_empty() {
            return Err(
                "webhook cadence requires a path after 'webhook:', e.g. 'webhook:github'"
                    .to_string(),
            );
        }
        Ok(MissionCadence::Webhook { path, secret: None })
    } else if lower.split_whitespace().count() >= 5 {
        // Looks like a cron expression (5+ fields). `split_whitespace` handles
        // tabs and newlines, not just spaces.
        Ok(MissionCadence::Cron {
            expression: s.trim().to_string(),
            timezone,
        })
    } else {
        Err(format!(
            "unrecognized cadence '{s}'. Use 'manual', a cron expression \
             (e.g. '0 9 * * *'), 'event:<channel>:<pattern>' \
             (e.g. 'event:telegram:.*'), or 'webhook:<path>'"
        ))
    }
}

/// Translation result from a `routine_*` call into mission_* dispatch.
///
/// `mission_action` is the canonical mission_* name to dispatch.
/// `mission_params` is the rewritten parameter object that mission_* expects.
/// `post_create_update`, when present and the action is `mission_create`, is
/// applied via `MissionManager::update_mission` immediately after creation
/// to set fields that mission_create's signature does not accept directly
/// (description, context_paths, notify_user, cooldown_secs, max_concurrent,
/// dedup_window_secs).
#[derive(Debug, Clone)]
struct RoutineMissionAlias {
    mission_action: &'static str,
    mission_params: serde_json::Value,
    post_create_update: Option<ironclaw_engine::MissionUpdate>,
}

/// Translate a `routine_*` action call into mission_* parameters. Returns
/// `None` if `action_name` is not a routine alias.
fn routine_to_mission_alias(
    action_name: &str,
    params: &serde_json::Value,
) -> Option<RoutineMissionAlias> {
    match action_name {
        "routine_create" => {
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unnamed routine")
                .to_string();
            // Routines call the body field "prompt"; missions call it "goal".
            let goal = params
                .get("prompt")
                .or_else(|| params.get("goal"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let description = params
                .get("description")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);

            // Translate the routine `request` block into a MissionCadence
            // serialized as the cadence string parse_cadence understands. We
            // serialize as a structured string when possible, otherwise we
            // hand the cadence variant directly through metadata that
            // mission_create can't read — so we instead build the cadence
            // here and store it via the post_create_update path.
            let cadence = parse_routine_request(params);
            // We carry cadence + the new fields via the update path so we
            // don't need to change mission_create's flat-args contract.
            let mut updates = ironclaw_engine::MissionUpdate {
                description: description.clone(),
                ..Default::default()
            };
            updates.cadence = Some(cadence);

            // execution.context_paths
            if let Some(arr) = params
                .get("execution")
                .and_then(|e| e.get("context_paths"))
                .and_then(|v| v.as_array())
            {
                updates.context_paths = Some(
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                );
            }

            // delivery.user
            if let Some(user) = params
                .get("delivery")
                .and_then(|d| d.get("user"))
                .and_then(|v| v.as_str())
            {
                updates.notify_user = Some(user.to_string());
            }

            // delivery.channel — feeds notify_channels
            let mut notify_channels: Vec<String> = Vec::new();
            if let Some(ch) = params
                .get("delivery")
                .and_then(|d| d.get("channel"))
                .and_then(|v| v.as_str())
            {
                notify_channels.push(ch.to_string());
            }

            // advanced.cooldown_secs (also accepts top-level cooldown_secs)
            if let Some(secs) = params
                .get("advanced")
                .and_then(|a| a.get("cooldown_secs"))
                .or_else(|| params.get("cooldown_secs"))
                .and_then(|v| v.as_u64())
            {
                updates.cooldown_secs = Some(secs);
            }
            // guardrails.max_concurrent
            if let Some(max) = params
                .get("guardrails")
                .and_then(|g| g.get("max_concurrent"))
                .or_else(|| params.get("max_concurrent"))
                .and_then(|v| v.as_u64())
            {
                updates.max_concurrent = Some(max as u32);
            }
            // guardrails.dedup_window_secs
            if let Some(secs) = params
                .get("guardrails")
                .and_then(|g| g.get("dedup_window_secs"))
                .or_else(|| params.get("dedup_window_secs"))
                .and_then(|v| v.as_u64())
            {
                updates.dedup_window_secs = Some(secs);
            }

            // mission_create takes a `cadence` string as a flat param. We
            // pass "manual" here as a placeholder — the real cadence is
            // applied immediately afterward via update_mission. This keeps
            // the mission_create signature unchanged.
            let mut mission_params = serde_json::json!({
                "name": name,
                "goal": goal,
                "cadence": "manual",
            });
            if !notify_channels.is_empty()
                && let Some(obj) = mission_params.as_object_mut()
            {
                obj.insert(
                    "notify_channels".to_string(),
                    serde_json::json!(notify_channels),
                );
            }

            Some(RoutineMissionAlias {
                mission_action: "mission_create",
                mission_params,
                post_create_update: Some(updates),
            })
        }

        "routine_list" => Some(RoutineMissionAlias {
            mission_action: "mission_list",
            mission_params: params.clone(),
            post_create_update: None,
        }),

        "routine_fire" => Some(RoutineMissionAlias {
            mission_action: "mission_fire",
            mission_params: params.clone(),
            post_create_update: None,
        }),

        "routine_pause" => Some(RoutineMissionAlias {
            mission_action: "mission_pause",
            mission_params: params.clone(),
            post_create_update: None,
        }),

        "routine_resume" => Some(RoutineMissionAlias {
            mission_action: "mission_resume",
            mission_params: params.clone(),
            post_create_update: None,
        }),

        "routine_delete" => Some(RoutineMissionAlias {
            mission_action: "mission_complete",
            mission_params: params.clone(),
            post_create_update: None,
        }),

        "routine_update" => {
            // Mission_update accepts the same flat fields the routine API
            // already exposes (id, name, goal, cadence, notify_channels,
            // success_criteria) plus the new ones. Translate routine
            // execution/delivery/advanced/guardrails sub-objects into the
            // flat mission_update keys the existing arm reads.
            let mut translated = match params {
                serde_json::Value::Object(map) => map.clone(),
                _ => serde_json::Map::new(),
            };
            if let Some(prompt) = params.get("prompt").and_then(|v| v.as_str()) {
                translated.insert(
                    "goal".to_string(),
                    serde_json::Value::String(prompt.to_string()),
                );
            }
            if let Some(arr) = params
                .get("execution")
                .and_then(|e| e.get("context_paths"))
                .cloned()
            {
                translated.insert("context_paths".to_string(), arr);
            }
            if let Some(user) = params.get("delivery").and_then(|d| d.get("user")).cloned() {
                translated.insert("notify_user".to_string(), user);
            }
            if let Some(ch) = params
                .get("delivery")
                .and_then(|d| d.get("channel"))
                .and_then(|v| v.as_str())
            {
                translated.insert(
                    "notify_channels".to_string(),
                    serde_json::json!([ch.to_string()]),
                );
            }
            if let Some(secs) = params
                .get("advanced")
                .and_then(|a| a.get("cooldown_secs"))
                .cloned()
            {
                translated.insert("cooldown_secs".to_string(), secs);
            }
            if let Some(secs) = params
                .get("guardrails")
                .and_then(|g| g.get("dedup_window_secs"))
                .cloned()
            {
                translated.insert("dedup_window_secs".to_string(), secs);
            }
            if let Some(max) = params
                .get("guardrails")
                .and_then(|g| g.get("max_concurrent"))
                .cloned()
            {
                translated.insert("max_concurrent".to_string(), max);
            }
            // Cadence: derive from the request block if present.
            if params.get("request").is_some() {
                let cadence = parse_routine_request(params);
                // We can't pass a structured cadence through the
                // mission_update arm, which only reads a "cadence" string.
                // Encode it back into the cadence string the parser
                // recognizes (cron expr / "event:..." / "webhook:..." /
                // "manual"). Structured filters and channel filters that
                // can't round-trip into a string fall back through the
                // post-create update path on `routine_create`, but for
                // `routine_update` we can't fully express them today —
                // log a debug and drop the structured pieces.
                let cadence_str = cadence_to_round_trip_string(&cadence);
                translated.insert(
                    "cadence".to_string(),
                    serde_json::Value::String(cadence_str),
                );
            }

            Some(RoutineMissionAlias {
                mission_action: "mission_update",
                mission_params: serde_json::Value::Object(translated),
                post_create_update: None,
            })
        }

        _ => None,
    }
}

/// Parse the routine `request` sub-object into a `MissionCadence`.
/// Falls back to `Manual` when the kind is missing or unrecognized.
fn parse_routine_request(
    params: &serde_json::Value,
) -> ironclaw_engine::types::mission::MissionCadence {
    use ironclaw_engine::types::mission::MissionCadence;

    let request = params.get("request");
    let kind = request
        .and_then(|r| r.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("manual");

    match kind {
        "cron" => MissionCadence::Cron {
            expression: request
                .and_then(|r| r.get("schedule"))
                .and_then(|v| v.as_str())
                .unwrap_or("0 0 * * * *")
                .to_string(),
            // Validate the timezone string at the bridge boundary so an
            // invalid value never enters the engine. An empty/invalid value
            // is silently dropped (None) — the engine then resolves the
            // schedule in UTC, matching the previous string-based behaviour
            // for unknown zones.
            timezone: request
                .and_then(|r| r.get("timezone"))
                .and_then(|v| v.as_str())
                .and_then(ironclaw_common::ValidTimezone::parse),
        },
        "message_event" => MissionCadence::OnEvent {
            event_pattern: request
                .and_then(|r| r.get("pattern"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            channel: request
                .and_then(|r| r.get("channel"))
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        "system_event" => {
            let mut filters = std::collections::HashMap::new();
            if let Some(map) = request
                .and_then(|r| r.get("filters"))
                .and_then(|v| v.as_object())
            {
                for (k, v) in map {
                    filters.insert(k.clone(), v.clone());
                }
            }
            MissionCadence::OnSystemEvent {
                source: request
                    .and_then(|r| r.get("source"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                event_type: request
                    .and_then(|r| r.get("event_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                filters,
            }
        }
        "webhook" => MissionCadence::Webhook {
            path: request
                .and_then(|r| r.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            secret: request
                .and_then(|r| r.get("secret"))
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        _ => MissionCadence::Manual,
    }
}

/// Encode a `MissionCadence` into a string that `parse_cadence` can round-trip.
/// Structured features (channel filter, system event filters, webhook secret)
/// are lossy through this path; callers that need full fidelity should use
/// `update_mission` with a typed `MissionUpdate` instead.
fn cadence_to_round_trip_string(
    cadence: &ironclaw_engine::types::mission::MissionCadence,
) -> String {
    use ironclaw_engine::types::mission::MissionCadence;
    match cadence {
        MissionCadence::Cron { expression, .. } => expression.clone(),
        MissionCadence::OnEvent {
            event_pattern,
            channel,
        } => match channel {
            Some(ch) => format!("event:{ch}:{event_pattern}"),
            None => format!("event:*:{event_pattern}"),
        },
        MissionCadence::OnSystemEvent {
            source, event_type, ..
        } => {
            format!("system_event:{source}/{event_type}")
        }
        MissionCadence::Webhook { path, .. } => format!("webhook:{path}"),
        MissionCadence::Manual => "manual".to_string(),
    }
}

/// Extract credential name from an authentication_required error message.
///
/// The HTTP tool returns errors like:
/// `{"error":"authentication_required","credential_name":"github_token",...}`
fn extract_credential_name(error_msg: &str) -> Option<String> {
    // The error is JSON-encoded inside the tool error string.
    // Find the JSON portion and parse credential_name from it.
    if let Some(json_start) = error_msg.find('{')
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&error_msg[json_start..])
    {
        return parsed
            .get("credential_name")
            .and_then(|v| v.as_str())
            .map(String::from);
    }
    None
}

fn is_v1_only_tool(name: &str) -> bool {
    // routine_* tools are surfaced in v2 too, but are intercepted by
    // `handle_mission_call`'s routine alias path *before* this check fires —
    // they get translated into mission_* dispatches via the existing
    // mission manager rather than the v1 routine engine. The original v1
    // routine tools remain registered for the v1 engine, but in v2 the
    // alias path means the LLM-facing routine_create/list/update/etc.
    // calls always go through missions.
    matches!(
        name,
        "create_job"
            | "create-job"
            | "cancel_job"
            | "cancel-job"
            | "build_software"
            | "build-software"
    )
}

/// Auth management tools from v1 that are now kernel-internal in v2.
/// The LLM should not see or call these — auth is handled automatically.
fn is_v1_auth_tool(name: &str) -> bool {
    matches!(name, "tool_auth" | "tool-auth")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::JobContext;
    use crate::tools::{Tool, ToolError, ToolOutput};
    use async_trait::async_trait;

    fn make_adapter() -> EffectBridgeAdapter {
        use ironclaw_safety::SafetyConfig;
        let config = SafetyConfig {
            max_output_length: 10_000,
            injection_check_enabled: false,
        };
        EffectBridgeAdapter::new(
            Arc::new(ToolRegistry::new()),
            Arc::new(SafetyLayer::new(&config)),
            Arc::new(HookRegistry::default()),
        )
    }

    /// Verify that reset_call_count resets the counter to zero,
    /// preventing the "call limit reached" error across threads.
    #[test]
    fn call_count_resets_between_threads() {
        let adapter = make_adapter();

        // Simulate 50 tool calls (the limit)
        for _ in 0..50 {
            adapter
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        assert_eq!(
            adapter
                .call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            50
        );

        // Reset — simulates what handle_with_engine does before each thread
        adapter.reset_call_count();
        assert_eq!(
            adapter
                .call_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    /// Verify that auto_approve_tool adds entries and is queryable.
    #[tokio::test]
    async fn auto_approve_tracks_tools() {
        let adapter = make_adapter();

        assert!(!adapter.auto_approved.read().await.contains("shell"));
        adapter.auto_approve_tool("shell").await;
        assert!(adapter.auto_approved.read().await.contains("shell"));
    }

    #[tokio::test]
    async fn global_auto_approve_skips_unless_auto_approved_gates() {
        use ironclaw_safety::SafetyConfig;

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(ApprovalTestTool)).await;

        let adapter = EffectBridgeAdapter::new(
            tools,
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        )
        .with_global_auto_approve(true);

        let result = adapter
            .execute_action(
                "approval_test",
                serde_json::json!({"value": "x"}),
                &lease(),
                &exec_ctx(
                    ironclaw_engine::ThreadId::new(),
                    Some("call_global_auto_approve"),
                ),
            )
            .await
            .expect("global auto-approve should bypass approval gate");

        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn global_auto_approve_does_not_bypass_always_gates() {
        use ironclaw_safety::SafetyConfig;

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(AlwaysApprovalTestTool)).await;

        let adapter = EffectBridgeAdapter::new(
            tools,
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        )
        .with_global_auto_approve(true);

        let result = adapter
            .execute_action(
                "always_approval_test",
                serde_json::json!({"value": "x"}),
                &lease(),
                &exec_ctx(
                    ironclaw_engine::ThreadId::new(),
                    Some("call_global_auto_approve_always"),
                ),
            )
            .await;

        assert!(matches!(result, Err(EngineError::LeaseDenied { .. })));
    }

    struct ApprovalTestTool;

    struct AlwaysApprovalTestTool;

    #[async_trait]
    impl Tool for ApprovalTestTool {
        fn name(&self) -> &str {
            "approval_test"
        }

        fn description(&self) -> &str {
            "Test tool that requires approval"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(
                serde_json::json!({ "echo": params }),
                std::time::Duration::from_millis(1),
            ))
        }

        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::UnlessAutoApproved
        }
    }

    #[async_trait]
    impl Tool for AlwaysApprovalTestTool {
        fn name(&self) -> &str {
            "always_approval_test"
        }

        fn description(&self) -> &str {
            "Test tool that always requires explicit approval"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                }
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(
                serde_json::json!({ "echo": params }),
                std::time::Duration::from_millis(1),
            ))
        }

        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::Always
        }
    }

    fn lease() -> ironclaw_engine::CapabilityLease {
        ironclaw_engine::CapabilityLease {
            id: ironclaw_engine::types::capability::LeaseId::new(),
            thread_id: ironclaw_engine::ThreadId::new(),
            capability_name: "tools".into(),
            granted_actions: ironclaw_engine::GrantedActions::All,
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        }
    }

    fn exec_ctx(
        thread_id: ironclaw_engine::ThreadId,
        call_id: Option<&str>,
    ) -> ironclaw_engine::ThreadExecutionContext {
        ironclaw_engine::ThreadExecutionContext {
            thread_id,
            thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
            project_id: ironclaw_engine::ProjectId::new(),
            user_id: "test_user".to_string(),
            step_id: ironclaw_engine::StepId::new(),
            current_call_id: call_id.map(str::to_string),
            source_channel: None,
            user_timezone: None,
        }
    }

    #[tokio::test]
    async fn need_approval_preserves_current_call_id() {
        use ironclaw_safety::SafetyConfig;

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(ApprovalTestTool)).await;

        let adapter = EffectBridgeAdapter::new(
            Arc::clone(&tools),
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );

        let thread_id = ironclaw_engine::ThreadId::new();
        let result = adapter
            .execute_action(
                "approval_test",
                serde_json::json!({"value": "x"}),
                &lease(),
                &exec_ctx(thread_id, Some("call_approve_1")),
            )
            .await;

        match result {
            Err(EngineError::GatePaused {
                call_id, gate_name, ..
            }) => {
                assert_eq!(call_id, "call_approve_1");
                assert_eq!(gate_name, "approval");
            }
            other => panic!("expected GatePaused, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolved_pending_action_bypasses_approval_once() {
        use ironclaw_safety::SafetyConfig;

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(ApprovalTestTool)).await;

        let adapter = EffectBridgeAdapter::new(
            tools,
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );

        let thread_id = ironclaw_engine::ThreadId::new();
        let first = adapter
            .execute_action(
                "approval_test",
                serde_json::json!({"value": "x"}),
                &lease(),
                &exec_ctx(thread_id, Some("call_once_1")),
            )
            .await;
        assert!(matches!(first, Err(EngineError::GatePaused { .. })));

        let second = adapter
            .execute_resolved_pending_action(
                "approval_test",
                serde_json::json!({"value": "x"}),
                &lease(),
                &exec_ctx(thread_id, Some("call_once_1")),
                true,
            )
            .await
            .expect("resolved pending action should bypass approval");
        assert!(!second.is_error);

        let third = adapter
            .execute_action(
                "approval_test",
                serde_json::json!({"value": "y"}),
                &lease(),
                &exec_ctx(thread_id, Some("call_once_2")),
            )
            .await;
        assert!(matches!(third, Err(EngineError::GatePaused { .. })));
    }

    /// Regression for nearai/ironclaw#2206: a `tool_activate`/`tool_auth`
    /// extension result containing a non-https `auth_url` (e.g.
    /// `javascript:alert(1)`) must be sanitized to `None` before it reaches
    /// `ResumeKind::Authentication` and is forwarded onto the gate stream.
    ///
    /// This test deliberately drives `EffectBridgeAdapter::execute_action`
    /// (the call site) instead of `auth_gate_from_extension_result` in
    /// isolation, per the "Test Through the Caller, Not Just the Helper"
    /// rule in `.claude/rules/testing.md`.
    #[tokio::test]
    async fn auth_gate_strips_non_https_auth_url_from_tool_activate_output() {
        use ironclaw_safety::SafetyConfig;

        struct OAuthPromptTool;

        #[async_trait]
        impl Tool for OAuthPromptTool {
            fn name(&self) -> &str {
                "tool_activate"
            }

            fn description(&self) -> &str {
                "Test stub for tool_activate that returns a malicious auth_url"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }

            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &JobContext,
            ) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "awaiting_authorization",
                        "name": "evil_ext",
                        "instructions": "Complete sign-in",
                        "auth_url": "javascript:alert(1)",
                    }),
                    std::time::Duration::from_millis(1),
                ))
            }

            fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
                ApprovalRequirement::Never
            }
        }

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(OAuthPromptTool)).await;

        let adapter = EffectBridgeAdapter::new(
            tools,
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );

        let result = adapter
            .execute_action(
                "tool_activate",
                serde_json::json!({}),
                &lease(),
                &exec_ctx(
                    ironclaw_engine::ThreadId::new(),
                    Some("call_auth_url_sanitize"),
                ),
            )
            .await;

        match result {
            Err(EngineError::GatePaused {
                gate_name,
                resume_kind,
                ..
            }) => {
                assert_eq!(gate_name, "authentication");
                match *resume_kind {
                    ironclaw_engine::ResumeKind::Authentication { auth_url, .. } => {
                        assert!(
                            auth_url.is_none(),
                            "javascript: auth_url must be stripped before reaching ResumeKind, got {auth_url:?}"
                        );
                    }
                    other => panic!("expected Authentication resume kind, got {other:?}"),
                }
            }
            other => {
                panic!("expected GatePaused(authentication), got {other:?}")
            }
        }
    }

    /// Sibling regression: a well-formed `https://` auth_url must still
    /// flow through unmodified. Guards against an over-eager sanitizer.
    #[tokio::test]
    async fn auth_gate_preserves_https_auth_url_from_tool_activate_output() {
        use ironclaw_safety::SafetyConfig;

        struct OAuthPromptTool;

        #[async_trait]
        impl Tool for OAuthPromptTool {
            fn name(&self) -> &str {
                "tool_activate"
            }

            fn description(&self) -> &str {
                "Test stub for tool_activate that returns a valid auth_url"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }

            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &JobContext,
            ) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "awaiting_authorization",
                        "name": "good_ext",
                        "instructions": "Complete sign-in",
                        "auth_url": "https://accounts.google.com/o/oauth2/auth",
                    }),
                    std::time::Duration::from_millis(1),
                ))
            }

            fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
                ApprovalRequirement::Never
            }
        }

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(OAuthPromptTool)).await;

        let adapter = EffectBridgeAdapter::new(
            tools,
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );

        let result = adapter
            .execute_action(
                "tool_activate",
                serde_json::json!({}),
                &lease(),
                &exec_ctx(
                    ironclaw_engine::ThreadId::new(),
                    Some("call_auth_url_passthrough"),
                ),
            )
            .await;

        match result {
            Err(EngineError::GatePaused { resume_kind, .. }) => match *resume_kind {
                ironclaw_engine::ResumeKind::Authentication { auth_url, .. } => {
                    assert_eq!(
                        auth_url.as_deref(),
                        Some("https://accounts.google.com/o/oauth2/auth"),
                    );
                }
                other => panic!("expected Authentication resume kind, got {other:?}"),
            },
            other => panic!("expected GatePaused(authentication), got {other:?}"),
        }
    }

    // ── routine→mission alias tests ────────────────────────────

    #[test]
    fn routine_create_alias_translates_cron_with_full_field_set() {
        let params = serde_json::json!({
            "name": "Daily PR digest",
            "prompt": "Summarize open PRs needing review",
            "description": "Morning developer briefing",
            "request": {
                "kind": "cron",
                "schedule": "0 9 * * *",
                "timezone": "America/New_York",
            },
            "execution": {
                "context_paths": ["context/profile.json", "MEMORY.md"],
            },
            "delivery": {
                "channel": "gateway",
                "user": "alice",
            },
            "advanced": {
                "cooldown_secs": 300,
            },
            "guardrails": {
                "max_concurrent": 1,
                "dedup_window_secs": 60,
            },
        });

        let alias = routine_to_mission_alias("routine_create", &params)
            .expect("routine_create should produce an alias");
        assert_eq!(alias.mission_action, "mission_create");
        assert_eq!(
            alias.mission_params.get("name").and_then(|v| v.as_str()),
            Some("Daily PR digest")
        );
        assert_eq!(
            alias.mission_params.get("goal").and_then(|v| v.as_str()),
            Some("Summarize open PRs needing review")
        );
        // mission_create receives a placeholder cadence; the real cadence is
        // applied via the post_create_update.
        assert_eq!(
            alias.mission_params.get("cadence").and_then(|v| v.as_str()),
            Some("manual")
        );
        assert_eq!(
            alias
                .mission_params
                .get("notify_channels")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
            Some(vec!["gateway"])
        );

        let updates = alias
            .post_create_update
            .expect("routine_create should populate updates");
        assert_eq!(
            updates.description.as_deref(),
            Some("Morning developer briefing")
        );
        assert_eq!(
            updates.context_paths.as_deref(),
            // safety: array-to-slice coercion, not a string byte slice
            Some(&["context/profile.json".to_string(), "MEMORY.md".to_string()][..])
        );
        assert_eq!(updates.notify_user.as_deref(), Some("alice"));
        assert_eq!(updates.cooldown_secs, Some(300));
        assert_eq!(updates.max_concurrent, Some(1));
        assert_eq!(updates.dedup_window_secs, Some(60));
        match updates.cadence.as_ref().expect("cadence in updates") {
            ironclaw_engine::types::mission::MissionCadence::Cron {
                expression,
                timezone,
            } => {
                assert_eq!(expression, "0 9 * * *");
                assert_eq!(
                    timezone.as_ref().map(|tz| tz.tz().name()),
                    Some("America/New_York")
                );
            }
            other => panic!("expected Cron cadence, got {:?}", other),
        }
    }

    #[test]
    fn routine_create_alias_translates_message_event_with_channel_filter() {
        let params = serde_json::json!({
            "name": "GitHub PR watcher",
            "prompt": "React to PR review requests",
            "request": {
                "kind": "message_event",
                "pattern": "review requested",
                "channel": "github",
            },
        });
        let alias =
            routine_to_mission_alias("routine_create", &params).expect("alias for message_event");
        let updates = alias.post_create_update.expect("updates");
        match updates.cadence.as_ref().expect("cadence") {
            ironclaw_engine::types::mission::MissionCadence::OnEvent {
                event_pattern,
                channel,
            } => {
                assert_eq!(event_pattern, "review requested");
                assert_eq!(channel.as_deref(), Some("github"));
            }
            other => panic!("expected OnEvent cadence, got {:?}", other),
        }
    }

    #[test]
    fn routine_create_alias_translates_system_event_with_filters() {
        let params = serde_json::json!({
            "name": "Issue triage",
            "prompt": "Triage opened issues",
            "request": {
                "kind": "system_event",
                "source": "github",
                "event_type": "issue.opened",
                "filters": {
                    "repository_name": "nearai/ironclaw",
                    "sender_login": "ilblackdragon",
                },
            },
        });
        let alias = routine_to_mission_alias("routine_create", &params).expect("alias");
        let updates = alias.post_create_update.expect("updates");
        match updates.cadence.as_ref().expect("cadence") {
            ironclaw_engine::types::mission::MissionCadence::OnSystemEvent {
                source,
                event_type,
                filters,
            } => {
                assert_eq!(source, "github");
                assert_eq!(event_type, "issue.opened");
                assert_eq!(filters.len(), 2);
                assert_eq!(
                    filters.get("repository_name").and_then(|v| v.as_str()),
                    Some("nearai/ironclaw")
                );
                assert_eq!(
                    filters.get("sender_login").and_then(|v| v.as_str()),
                    Some("ilblackdragon")
                );
            }
            other => panic!("expected OnSystemEvent cadence, got {:?}", other),
        }
    }

    #[test]
    fn routine_create_alias_translates_webhook() {
        let params = serde_json::json!({
            "name": "GitHub webhook",
            "prompt": "Handle inbound GitHub events",
            "request": {
                "kind": "webhook",
                "path": "github",
                "secret": "shh",
            },
        });
        let alias = routine_to_mission_alias("routine_create", &params).expect("alias");
        let updates = alias.post_create_update.expect("updates");
        match updates.cadence.as_ref().expect("cadence") {
            ironclaw_engine::types::mission::MissionCadence::Webhook { path, secret } => {
                assert_eq!(path, "github");
                assert_eq!(secret.as_deref(), Some("shh"));
            }
            other => panic!("expected Webhook cadence, got {:?}", other),
        }
    }

    #[test]
    fn parse_cadence_event_channel_pattern_format() {
        // event:<channel>:<pattern> should populate both fields.
        let cadence = parse_cadence("event:telegram:.*", None).expect("should parse");
        match cadence {
            ironclaw_engine::types::mission::MissionCadence::OnEvent {
                event_pattern,
                channel,
            } => {
                assert_eq!(event_pattern, ".*");
                assert_eq!(channel.as_deref(), Some("telegram"));
            }
            other => panic!("expected OnEvent, got {other:?}"),
        }

        // Pattern with special regex chars.
        let cadence = parse_cadence("event:github:review requested", None).expect("should parse");
        match cadence {
            ironclaw_engine::types::mission::MissionCadence::OnEvent {
                event_pattern,
                channel,
            } => {
                assert_eq!(event_pattern, "review requested");
                assert_eq!(channel.as_deref(), Some("github"));
            }
            other => panic!("expected OnEvent, got {other:?}"),
        }

        // Pattern containing colons (split on first colon only).
        let cadence = parse_cadence("event:slack:error:.*fatal", None).expect("should parse");
        match cadence {
            ironclaw_engine::types::mission::MissionCadence::OnEvent {
                event_pattern,
                channel,
            } => {
                assert_eq!(event_pattern, "error:.*fatal");
                assert_eq!(channel.as_deref(), Some("slack"));
            }
            other => panic!("expected OnEvent, got {other:?}"),
        }
    }

    #[test]
    fn parse_cadence_event_rejects_missing_channel_or_pattern() {
        // Just "event:<something>" with no second colon should fail.
        let err = parse_cadence("event:telegram", None).unwrap_err();
        assert!(err.contains("event:<channel>:<pattern>"), "got: {err}");

        // Empty channel.
        let err = parse_cadence("event::.*", None).unwrap_err();
        assert!(err.contains("event:<channel>:<pattern>"), "got: {err}");

        // Empty pattern.
        let err = parse_cadence("event:telegram:", None).unwrap_err();
        assert!(err.contains("event:<channel>:<pattern>"), "got: {err}");
    }

    #[test]
    fn parse_cadence_event_rejects_invalid_regex() {
        let err = parse_cadence("event:telegram:[invalid(", None).unwrap_err();
        assert!(err.contains("not a valid regex"), "got: {err}");
    }

    #[test]
    fn parse_cadence_event_prefix_wins_over_cron_heuristic() {
        // Regression: an event cadence with 5+ whitespace-separated tokens
        // in the pattern must NOT be misclassified as a cron expression.
        let cadence =
            parse_cadence("event:slack:a]b c d e f", None).expect("should parse as event");
        assert!(matches!(
            cadence,
            ironclaw_engine::types::mission::MissionCadence::OnEvent { .. }
        ));

        // Same hazard for `webhook:` — verify the prefix wins.
        let cadence = parse_cadence("webhook: a b c d e", None).expect("should parse");
        assert!(matches!(
            cadence,
            ironclaw_engine::types::mission::MissionCadence::Webhook { .. }
        ));

        // Sanity: a real cron expression still parses as cron.
        let cadence = parse_cadence("0 9 * * *", None).expect("should parse");
        assert!(matches!(
            cadence,
            ironclaw_engine::types::mission::MissionCadence::Cron { .. }
        ));
    }

    #[test]
    fn routine_create_alias_defaults_to_manual_when_request_missing() {
        let params = serde_json::json!({
            "name": "Manual mission",
            "prompt": "Run on demand",
        });
        let alias = routine_to_mission_alias("routine_create", &params).expect("alias");
        let updates = alias.post_create_update.expect("updates");
        match updates.cadence.as_ref().expect("cadence") {
            ironclaw_engine::types::mission::MissionCadence::Manual => {}
            other => panic!("expected Manual cadence, got {:?}", other),
        }
    }

    #[test]
    fn routine_simple_actions_alias_to_mission_counterparts() {
        let params = serde_json::json!({"id": "00000000-0000-0000-0000-000000000000"});
        for (routine, mission) in &[
            ("routine_list", "mission_list"),
            ("routine_fire", "mission_fire"),
            ("routine_pause", "mission_pause"),
            ("routine_resume", "mission_resume"),
            ("routine_delete", "mission_complete"),
        ] {
            let alias = routine_to_mission_alias(routine, &params)
                .unwrap_or_else(|| panic!("expected alias for {routine}"));
            assert_eq!(alias.mission_action, *mission, "wrong target for {routine}");
            assert!(alias.post_create_update.is_none());
        }
    }

    #[test]
    fn routine_update_alias_translates_nested_to_flat() {
        let params = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "prompt": "Updated goal",
            "execution": {
                "context_paths": ["NOTES.md"],
            },
            "delivery": {
                "channel": "repl",
                "user": "bob",
            },
            "advanced": {"cooldown_secs": 600},
            "guardrails": {"dedup_window_secs": 120, "max_concurrent": 2},
            "request": {
                "kind": "cron",
                "schedule": "0 12 * * *",
            },
        });
        let alias = routine_to_mission_alias("routine_update", &params).expect("alias");
        assert_eq!(alias.mission_action, "mission_update");
        let mp = &alias.mission_params;
        assert_eq!(
            mp.get("goal").and_then(|v| v.as_str()),
            Some("Updated goal")
        );
        assert_eq!(mp.get("notify_user").and_then(|v| v.as_str()), Some("bob"));
        assert_eq!(
            mp.get("notify_channels")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str()),
            Some("repl")
        );
        assert_eq!(mp.get("cooldown_secs").and_then(|v| v.as_u64()), Some(600));
        assert_eq!(
            mp.get("dedup_window_secs").and_then(|v| v.as_u64()),
            Some(120)
        );
        assert_eq!(mp.get("max_concurrent").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(
            mp.get("context_paths")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str()),
            Some("NOTES.md")
        );
        assert_eq!(
            mp.get("cadence").and_then(|v| v.as_str()),
            Some("0 12 * * *")
        );
    }

    #[test]
    fn extract_guardrails_rejects_string_typed_integers() {
        // Regression: LLMs pass numeric params as strings (e.g. cooldown_secs="0").
        // The old code silently ignored the wrong type, so mission_update
        // returned {"status":"updated"} but changed nothing in the database.
        let params = serde_json::json!({"cooldown_secs": "0", "max_concurrent": "2"});
        let mut updates = ironclaw_engine::MissionUpdate::default();
        let err = extract_guardrails(&params, &mut updates).unwrap_err();
        assert!(err.contains("must be an integer"), "got: {err}");

        // Integer values must succeed.
        let params = serde_json::json!({"cooldown_secs": 0, "max_concurrent": 2});
        let mut updates = ironclaw_engine::MissionUpdate::default();
        extract_guardrails(&params, &mut updates).expect("should succeed");
        assert_eq!(updates.cooldown_secs, Some(0));
        assert_eq!(updates.max_concurrent, Some(2));
    }

    #[test]
    fn parse_cadence_rejects_malformed_string() {
        // Regression: malformed cadence used to silently default to Manual,
        // causing reactive missions to never fire.
        let err = parse_cadence("bogus", None).unwrap_err();
        assert!(
            err.contains("unrecognized cadence"),
            "expected helpful error, got: {err}"
        );

        let err = parse_cadence("every 5 min", None).unwrap_err();
        assert!(err.contains("unrecognized cadence"));
    }

    #[test]
    fn parse_cadence_rejects_bare_event_prefix() {
        let err = parse_cadence("event:", None).unwrap_err();
        assert!(err.contains("event:<channel>:<pattern>"), "got: {err}");
    }

    #[test]
    fn parse_cadence_system_event_round_trips() {
        let cadence = parse_cadence("system_event:self-improvement/thread_completed", None)
            .expect("should parse system_event cadence");
        assert!(matches!(
            cadence,
            ironclaw_engine::types::mission::MissionCadence::OnSystemEvent {
                ref source,
                ref event_type,
                ..
            } if source == "self-improvement" && event_type == "thread_completed"
        ));
    }

    #[test]
    fn parse_cadence_rejects_malformed_system_event() {
        let err = parse_cadence("system_event:", None).unwrap_err();
        assert!(
            err.contains("system_event:<source>/<event_type>"),
            "got: {err}"
        );

        let err = parse_cadence("system_event:no_slash_here", None).unwrap_err();
        assert!(
            err.contains("system_event:<source>/<event_type>"),
            "got: {err}"
        );
    }

    #[test]
    fn parse_cadence_rejects_empty_webhook_path() {
        let err = parse_cadence("webhook:", None).unwrap_err();
        assert!(err.contains("requires a path"));
    }

    #[test]
    fn parse_cadence_accepts_manual() {
        let cadence = parse_cadence("manual", None).expect("should parse");
        assert!(matches!(
            cadence,
            ironclaw_engine::types::mission::MissionCadence::Manual
        ));
    }

    #[test]
    fn routine_alias_returns_none_for_unrelated_action() {
        let params = serde_json::json!({});
        assert!(routine_to_mission_alias("http", &params).is_none());
        assert!(routine_to_mission_alias("mission_create", &params).is_none());
        assert!(routine_to_mission_alias("web_search", &params).is_none());
    }

    // ── extract_credential_name tests ──────────────────────────

    #[test]
    fn extract_credential_from_auth_required_error() {
        let msg = r#"Tool 'http' failed: execution failed: {"error":"authentication_required","credential_name":"github_token","message":"Credential 'github_token' is not configured."}"#;
        assert_eq!(
            extract_credential_name(msg),
            Some("github_token".to_string())
        );
    }

    #[test]
    fn extract_credential_from_nested_json() {
        let msg = r#"Tool 'http' failed: {"error":"authentication_required","credential_name":"linear_api_key","message":"Use auth_setup"}"#;
        assert_eq!(
            extract_credential_name(msg),
            Some("linear_api_key".to_string())
        );
    }

    #[test]
    fn extract_credential_returns_none_for_non_auth_error() {
        let msg = "Tool 'http' failed: connection timeout";
        assert_eq!(extract_credential_name(msg), None);
    }

    #[test]
    fn extract_credential_returns_none_for_json_without_credential() {
        let msg = r#"Tool 'http' failed: {"error":"not_found","message":"404"}"#;
        assert_eq!(extract_credential_name(msg), None);
    }

    // ── is_v1_only_tool tests ──────────────────────────────────

    /// Routines are no longer classified as v1-only: in v2 they are
    /// surfaced to the LLM and intercepted by the routine→mission alias
    /// path in `handle_mission_call` *before* the v1-only check fires.
    /// The original v1 routine tools remain registered for the v1 engine.
    #[test]
    fn routine_tools_are_not_v1_only() {
        assert!(!is_v1_only_tool("routine_create"));
        assert!(!is_v1_only_tool("routine_list"));
        assert!(!is_v1_only_tool("routine_fire"));
        assert!(!is_v1_only_tool("routine_delete"));
        assert!(!is_v1_only_tool("routine_pause"));
        assert!(!is_v1_only_tool("routine_resume"));
        assert!(!is_v1_only_tool("routine_update"));
    }

    #[test]
    fn job_and_build_tools_remain_v1_only() {
        assert!(is_v1_only_tool("create_job"));
        assert!(is_v1_only_tool("cancel_job"));
        assert!(is_v1_only_tool("build_software"));
    }

    #[test]
    fn mission_tools_are_not_v1_only() {
        assert!(!is_v1_only_tool("mission_create"));
        assert!(!is_v1_only_tool("mission_list"));
        assert!(!is_v1_only_tool("mission_fire"));
        assert!(!is_v1_only_tool("http"));
        assert!(!is_v1_only_tool("web_search"));
    }

    // ── is_v1_auth_tool tests ─────────────────────────────────

    #[test]
    fn auth_tools_are_v1_auth() {
        assert!(is_v1_auth_tool("tool_auth"));
        assert!(is_v1_auth_tool("tool-auth"));
        assert!(!is_v1_auth_tool("tool_activate"));
        assert!(!is_v1_auth_tool("tool-activate"));
    }

    #[test]
    fn non_auth_tools_are_not_v1_auth() {
        assert!(!is_v1_auth_tool("tool_install"));
        assert!(!is_v1_auth_tool("tool-install"));
        assert!(!is_v1_auth_tool("http"));
        assert!(!is_v1_auth_tool("tool_search"));
        assert!(!is_v1_auth_tool("tool_list"));
    }

    // ── Pre-flight auth gate integration test ─────────────────

    #[tokio::test]
    async fn preflight_gate_blocks_missing_credential() {
        use crate::secrets::CredentialMapping;
        use crate::testing::credentials::test_secrets_store;
        use crate::tools::wasm::SharedCredentialRegistry;

        let secrets = Arc::new(test_secrets_store());
        let cred_reg = Arc::new(SharedCredentialRegistry::new());
        cred_reg.add_mappings(vec![CredentialMapping::bearer(
            "github_token",
            "api.github.com",
        )]);

        // Build adapter with credential registry
        let tools =
            Arc::new(ToolRegistry::new().with_credentials(Arc::clone(&cred_reg), secrets.clone()));
        tools.register_builtin_tools();

        use ironclaw_safety::SafetyConfig;
        let adapter = EffectBridgeAdapter::new(
            Arc::clone(&tools),
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );

        // Set auth manager
        let auth_mgr = Arc::new(AuthManager::new(
            secrets,
            None,
            None,
            Some(Arc::clone(&tools)),
        ));
        adapter.set_auth_manager(auth_mgr).await;

        // Verify adapter has both dependencies
        assert!(
            adapter.auth_manager.read().await.is_some(),
            "auth_manager should be set"
        );
        assert!(
            adapter.tools.credential_registry().is_some(),
            "credential_registry should be set"
        );

        // Call execute_action with http tool params pointing to api.github.com
        let params = serde_json::json!({
            "url": "https://api.github.com/repos/nearai/ironclaw/issues",
            "method": "GET"
        });
        let lease = ironclaw_engine::CapabilityLease {
            id: ironclaw_engine::types::capability::LeaseId::new(),
            thread_id: ironclaw_engine::ThreadId::new(),
            capability_name: "tools".into(),
            granted_actions: ironclaw_engine::GrantedActions::All,
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        };
        let ctx = ironclaw_engine::ThreadExecutionContext {
            thread_id: ironclaw_engine::ThreadId::new(),
            thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
            project_id: ironclaw_engine::ProjectId::new(),
            user_id: "test_user".to_string(),
            step_id: ironclaw_engine::StepId::new(),
            current_call_id: None,
            source_channel: None,
            user_timezone: None,
        };

        let result = adapter.execute_action("http", params, &lease, &ctx).await;

        // Auth preflight runs before the approval check in the adapter
        // pipeline (see the order of `auth_manager.check_action_auth` vs
        // `tool.requires_approval` in `execute_action`), so a missing-credential
        // HTTP call surfaces an Authentication gate before any approval gate.
        match result {
            Err(EngineError::GatePaused { resume_kind, .. }) => match *resume_kind {
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name, ..
                } => {
                    assert_eq!(credential_name, "github_token");
                }
                other => panic!("Expected Authentication gate, got: {other:?}"),
            },
            other => {
                panic!("Expected GatePaused for authentication preflight, got: {other:?}");
            }
        }
    }

    #[tokio::test]
    async fn tool_activate_awaiting_authorization_becomes_auth_gate() {
        struct ActivateTool;

        #[async_trait]
        impl Tool for ActivateTool {
            fn name(&self) -> &str {
                "tool_activate"
            }

            fn description(&self) -> &str {
                "activate"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"}
                    }
                })
            }

            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput::success(
                    serde_json::json!({
                        "name": "notion",
                        "status": "awaiting_authorization",
                        "auth_url": "https://example.com/oauth",
                    }),
                    std::time::Duration::from_millis(1),
                ))
            }
        }

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(ActivateTool)).await;

        let adapter = EffectBridgeAdapter::new(
            tools,
            Arc::new(SafetyLayer::new(&ironclaw_safety::SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );

        let lease = ironclaw_engine::CapabilityLease {
            id: ironclaw_engine::types::capability::LeaseId::new(),
            thread_id: ironclaw_engine::ThreadId::new(),
            capability_name: "tools".into(),
            granted_actions: ironclaw_engine::GrantedActions::All,
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        };
        let ctx = ironclaw_engine::ThreadExecutionContext {
            thread_id: ironclaw_engine::ThreadId::new(),
            thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
            project_id: ironclaw_engine::ProjectId::new(),
            user_id: "test_user".to_string(),
            step_id: ironclaw_engine::StepId::new(),
            current_call_id: Some("call_123".to_string()),
            source_channel: None,
            user_timezone: None,
        };

        let result = adapter
            .execute_action(
                "tool_activate",
                serde_json::json!({"name": "notion"}),
                &lease,
                &ctx,
            )
            .await;

        match result {
            Err(EngineError::GatePaused {
                gate_name,
                action_name,
                resume_kind,
                ..
            }) => {
                assert_eq!(gate_name, "authentication");
                assert_eq!(action_name, "tool_activate");
                match *resume_kind {
                    ironclaw_engine::ResumeKind::Authentication {
                        credential_name,
                        auth_url,
                        ..
                    } => {
                        assert_eq!(credential_name, "notion");
                        assert_eq!(auth_url.as_deref(), Some("https://example.com/oauth"));
                    }
                    other => panic!("expected authentication resume kind, got {other:?}"),
                }
            }
            other => panic!("expected auth gate pause, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_install_post_install_auth_gate_preserves_secret_name_for_resume() {
        struct InstallTool;

        #[async_trait]
        impl Tool for InstallTool {
            fn name(&self) -> &str {
                "tool_install"
            }

            fn description(&self) -> &str {
                "install"
            }

            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"}
                    }
                })
            }

            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &crate::context::JobContext,
            ) -> Result<ToolOutput, ToolError> {
                Ok(ToolOutput::success(
                    serde_json::json!({
                        "name": "telegram",
                        "status": "awaiting_token",
                        "credential_name": "telegram_bot_token",
                        "instructions": "Enter your Telegram Bot API token (from @BotFather)",
                    }),
                    std::time::Duration::from_millis(1),
                ))
            }
        }

        let tools = Arc::new(ToolRegistry::new());
        tools.register(Arc::new(InstallTool)).await;

        let adapter = EffectBridgeAdapter::new(
            Arc::clone(&tools),
            Arc::new(SafetyLayer::new(&ironclaw_safety::SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );

        let lease = ironclaw_engine::CapabilityLease {
            id: ironclaw_engine::types::capability::LeaseId::new(),
            thread_id: ironclaw_engine::ThreadId::new(),
            capability_name: "tools".into(),
            granted_actions: ironclaw_engine::GrantedActions::All,
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        };
        let ctx = ironclaw_engine::ThreadExecutionContext {
            thread_id: ironclaw_engine::ThreadId::new(),
            thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
            project_id: ironclaw_engine::ProjectId::new(),
            user_id: "test_user".to_string(),
            step_id: ironclaw_engine::StepId::new(),
            current_call_id: Some("call_install".to_string()),
            source_channel: None,
            user_timezone: None,
        };

        let result = adapter
            .execute_action(
                "tool_install",
                serde_json::json!({"name": "telegram"}),
                &lease,
                &ctx,
            )
            .await;

        match result {
            Err(EngineError::GatePaused { resume_kind, .. }) => match *resume_kind {
                ironclaw_engine::ResumeKind::Authentication {
                    credential_name, ..
                } => {
                    assert_eq!(credential_name, "telegram_bot_token");
                }
                other => panic!("expected authentication resume kind, got {other:?}"),
            },
            other => panic!("expected auth gate pause after tool_install, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn available_actions_include_latent_inactive_provider_actions() {
        use crate::secrets::InMemorySecretsStore;
        use crate::secrets::SecretsCrypto;
        use crate::tools::mcp::process::McpProcessManager;
        use crate::tools::mcp::session::McpSessionManager;

        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("tools")).expect("tools dir");
        std::fs::write(
            dir.path().join("tools").join("latent_tool.wasm"),
            b"fake-wasm",
        )
        .expect("write wasm");
        std::fs::write(
            dir.path()
                .join("tools")
                .join("latent_tool.capabilities.json"),
            r#"{"description":"latent adapter test"}"#,
        )
        .expect("write capabilities");

        let key = secrecy::SecretString::from(crate::secrets::keychain::generate_master_key_hex());
        let crypto = Arc::new(SecretsCrypto::new(key).expect("crypto"));
        let secrets: Arc<dyn crate::secrets::SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));

        let tools = Arc::new(ToolRegistry::new());
        let ext_mgr = Arc::new(crate::extensions::ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(McpProcessManager::new()),
            Arc::clone(&secrets),
            Arc::clone(&tools),
            None,
            None,
            dir.path().join("tools"),
            dir.path().join("channels"),
            None,
            "test_user".to_string(),
            None,
            vec![],
        ));

        let adapter = EffectBridgeAdapter::new(
            Arc::clone(&tools),
            Arc::new(SafetyLayer::new(&ironclaw_safety::SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );
        adapter
            .set_auth_manager(Arc::new(AuthManager::new(
                secrets,
                None,
                Some(ext_mgr),
                Some(Arc::clone(&tools)),
            )))
            .await;

        let actions = adapter.available_actions(&[]).await.expect("actions");
        assert!(actions.iter().any(|action| action.name == "latent_tool"));
    }

    // ── Caller-level mission action tests ─────────────────────
    //
    // These drive execute_action("mission_create"/...) through the full
    // handle_mission_call path, per .claude/rules/testing.md.

    mod mission_store {
        use ironclaw_engine::types::mission::{Mission, MissionId, MissionStatus};
        use ironclaw_engine::types::thread::{Thread, ThreadId, ThreadState};
        use ironclaw_engine::{EngineError, ProjectId};
        use std::collections::HashMap;
        use tokio::sync::RwLock;

        pub(super) struct TestStore {
            threads: RwLock<HashMap<ThreadId, Thread>>,
            missions: RwLock<HashMap<MissionId, Mission>>,
        }

        impl TestStore {
            pub fn new() -> Self {
                Self {
                    threads: RwLock::new(HashMap::new()),
                    missions: RwLock::new(HashMap::new()),
                }
            }
        }

        #[async_trait::async_trait]
        impl ironclaw_engine::Store for TestStore {
            async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
                self.threads.write().await.insert(thread.id, thread.clone());
                Ok(())
            }
            async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
                Ok(self.threads.read().await.get(&id).cloned())
            }
            async fn list_threads(
                &self,
                _: ProjectId,
                _: &str,
            ) -> Result<Vec<Thread>, EngineError> {
                Ok(vec![])
            }
            async fn update_thread_state(
                &self,
                _: ThreadId,
                _: ThreadState,
            ) -> Result<(), EngineError> {
                Ok(())
            }
            async fn save_step(&self, _: &ironclaw_engine::Step) -> Result<(), EngineError> {
                Ok(())
            }
            async fn load_steps(
                &self,
                _: ThreadId,
            ) -> Result<Vec<ironclaw_engine::Step>, EngineError> {
                Ok(vec![])
            }
            async fn append_events(
                &self,
                _: &[ironclaw_engine::ThreadEvent],
            ) -> Result<(), EngineError> {
                Ok(())
            }
            async fn load_events(
                &self,
                _: ThreadId,
            ) -> Result<Vec<ironclaw_engine::ThreadEvent>, EngineError> {
                Ok(vec![])
            }
            async fn save_project(&self, _: &ironclaw_engine::Project) -> Result<(), EngineError> {
                Ok(())
            }
            async fn load_project(
                &self,
                _: ProjectId,
            ) -> Result<Option<ironclaw_engine::Project>, EngineError> {
                Ok(None)
            }
            async fn save_memory_doc(
                &self,
                _: &ironclaw_engine::MemoryDoc,
            ) -> Result<(), EngineError> {
                Ok(())
            }
            async fn load_memory_doc(
                &self,
                _: ironclaw_engine::DocId,
            ) -> Result<Option<ironclaw_engine::MemoryDoc>, EngineError> {
                Ok(None)
            }
            async fn list_memory_docs(
                &self,
                _: ProjectId,
                _: &str,
            ) -> Result<Vec<ironclaw_engine::MemoryDoc>, EngineError> {
                Ok(vec![])
            }
            async fn save_lease(
                &self,
                _: &ironclaw_engine::CapabilityLease,
            ) -> Result<(), EngineError> {
                Ok(())
            }
            async fn load_active_leases(
                &self,
                _: ThreadId,
            ) -> Result<Vec<ironclaw_engine::CapabilityLease>, EngineError> {
                Ok(vec![])
            }
            async fn revoke_lease(
                &self,
                _: ironclaw_engine::types::capability::LeaseId,
                _: &str,
            ) -> Result<(), EngineError> {
                Ok(())
            }
            async fn save_mission(&self, mission: &Mission) -> Result<(), EngineError> {
                self.missions
                    .write()
                    .await
                    .insert(mission.id, mission.clone());
                Ok(())
            }
            async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
                Ok(self.missions.read().await.get(&id).cloned())
            }
            async fn list_missions(
                &self,
                project_id: ProjectId,
                user_id: &str,
            ) -> Result<Vec<Mission>, EngineError> {
                Ok(self
                    .missions
                    .read()
                    .await
                    .values()
                    .filter(|m| m.project_id == project_id && m.user_id == user_id)
                    .cloned()
                    .collect())
            }
            async fn list_all_missions(
                &self,
                project_id: ProjectId,
            ) -> Result<Vec<Mission>, EngineError> {
                Ok(self
                    .missions
                    .read()
                    .await
                    .values()
                    .filter(|m| m.project_id == project_id)
                    .cloned()
                    .collect())
            }
            async fn update_mission_status(
                &self,
                id: MissionId,
                status: MissionStatus,
            ) -> Result<(), EngineError> {
                if let Some(m) = self.missions.write().await.get_mut(&id) {
                    m.status = status;
                }
                Ok(())
            }
        }
    }

    /// Build a MissionManager backed by an in-memory store and wire it
    /// into an EffectBridgeAdapter so tests can drive `execute_action`.
    async fn make_adapter_with_missions() -> EffectBridgeAdapter {
        use ironclaw_engine::{CapabilityRegistry, LeaseManager, PolicyEngine, ThreadManager};
        use ironclaw_safety::SafetyConfig;

        // Minimal LlmBackend mock — missions don't call the LLM in these tests.
        struct NoopLlm;
        #[async_trait]
        impl ironclaw_engine::LlmBackend for NoopLlm {
            async fn complete(
                &self,
                _: &[ironclaw_engine::ThreadMessage],
                _: &[ironclaw_engine::ActionDef],
                _: &ironclaw_engine::LlmCallConfig,
            ) -> Result<ironclaw_engine::LlmOutput, ironclaw_engine::EngineError> {
                Ok(ironclaw_engine::LlmOutput {
                    response: ironclaw_engine::types::step::LlmResponse::Text("done".into()),
                    usage: ironclaw_engine::types::step::TokenUsage::default(),
                })
            }
            fn model_name(&self) -> &str {
                "noop"
            }
        }

        // Minimal EffectExecutor mock.
        struct NoopEffects;
        #[async_trait]
        impl ironclaw_engine::EffectExecutor for NoopEffects {
            async fn execute_action(
                &self,
                _: &str,
                _: serde_json::Value,
                _: &ironclaw_engine::CapabilityLease,
                _: &ironclaw_engine::ThreadExecutionContext,
            ) -> Result<ironclaw_engine::ActionResult, ironclaw_engine::EngineError> {
                Ok(ironclaw_engine::ActionResult {
                    call_id: String::new(),
                    action_name: String::new(),
                    output: serde_json::json!({}),
                    is_error: false,
                    duration: std::time::Duration::from_millis(1),
                })
            }
            async fn available_actions(
                &self,
                _: &[ironclaw_engine::CapabilityLease],
            ) -> Result<Vec<ironclaw_engine::ActionDef>, ironclaw_engine::EngineError> {
                Ok(vec![])
            }
        }

        let store: Arc<dyn ironclaw_engine::Store> = Arc::new(mission_store::TestStore::new());
        let thread_manager = Arc::new(ThreadManager::new(
            Arc::new(NoopLlm),
            Arc::new(NoopEffects),
            Arc::clone(&store),
            Arc::new(CapabilityRegistry::new()),
            Arc::new(LeaseManager::new()),
            Arc::new(PolicyEngine::new()),
        ));
        let mgr = ironclaw_engine::MissionManager::new(Arc::clone(&store), thread_manager);

        let adapter = EffectBridgeAdapter::new(
            Arc::new(ToolRegistry::new()),
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );
        adapter.set_mission_manager(Arc::new(mgr)).await;
        adapter
    }

    /// Regression: mission_create with missing cadence must return an
    /// actionable error through the full execute_action path, not panic
    /// or silently create a Manual mission.
    #[tokio::test]
    async fn mission_create_missing_cadence_returns_error_via_execute_action() {
        let adapter = make_adapter_with_missions().await;
        let result = adapter
            .execute_action(
                "mission_create",
                serde_json::json!({"name": "test", "goal": "do stuff"}),
                &lease(),
                &exec_ctx(ironclaw_engine::ThreadId::new(), Some("c1")),
            )
            .await
            .expect("should return Ok with is_error=true, not Err");

        assert!(result.is_error, "missing cadence should be an error result");
        let output = &result.output;
        assert!(
            output
                .get("error")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("cadence is required")),
            "error message should mention cadence, got: {output}"
        );
    }

    /// Regression: mission_create with a malformed cadence string must
    /// return a helpful error through execute_action.
    #[tokio::test]
    async fn mission_create_malformed_cadence_returns_error_via_execute_action() {
        let adapter = make_adapter_with_missions().await;
        let result = adapter
            .execute_action(
                "mission_create",
                serde_json::json!({
                    "name": "test",
                    "goal": "do stuff",
                    "cadence": "every tuesday"
                }),
                &lease(),
                &exec_ctx(ironclaw_engine::ThreadId::new(), Some("c2")),
            )
            .await
            .expect("should return Ok with is_error=true");

        assert!(result.is_error);
        assert!(
            result
                .output
                .get("error")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("unrecognized cadence")),
            "got: {}",
            result.output
        );
    }

    /// Regression: mission_create with string-typed guardrails (e.g.
    /// cooldown_secs="0") must be caught before creating the mission.
    #[tokio::test]
    async fn mission_create_string_guardrails_rejected_via_execute_action() {
        let adapter = make_adapter_with_missions().await;
        let result = adapter
            .execute_action(
                "mission_create",
                serde_json::json!({
                    "name": "test",
                    "goal": "do stuff",
                    "cadence": "manual",
                    "cooldown_secs": "300"
                }),
                &lease(),
                &exec_ctx(ironclaw_engine::ThreadId::new(), Some("c3")),
            )
            .await
            .expect("should return Ok with is_error=true");

        assert!(result.is_error);
        assert!(
            result
                .output
                .get("error")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("must be an integer")),
            "got: {}",
            result.output
        );
    }

    /// Happy path: mission_create with valid params succeeds through execute_action.
    #[tokio::test]
    async fn mission_create_valid_params_succeeds_via_execute_action() {
        let adapter = make_adapter_with_missions().await;
        let result = adapter
            .execute_action(
                "mission_create",
                serde_json::json!({
                    "name": "daily check",
                    "goal": "check systems",
                    "cadence": "0 9 * * *"
                }),
                &lease(),
                &exec_ctx(ironclaw_engine::ThreadId::new(), Some("c4")),
            )
            .await
            .expect("should succeed");

        assert!(!result.is_error, "got error: {}", result.output);
        assert_eq!(
            result.output.get("status").and_then(|v| v.as_str()),
            Some("created")
        );
        assert!(
            result
                .output
                .get("mission_id")
                .and_then(|v| v.as_str())
                .is_some()
        );
    }

    /// Regression: mission_update with string-typed guardrails must be
    /// caught at the execute_action level, not silently ignored.
    #[tokio::test]
    async fn mission_update_string_guardrails_rejected_via_execute_action() {
        let adapter = make_adapter_with_missions().await;
        let ctx = exec_ctx(ironclaw_engine::ThreadId::new(), Some("u1"));

        // First create a mission to get an ID.
        let create_result = adapter
            .execute_action(
                "mission_create",
                serde_json::json!({
                    "name": "updatable",
                    "goal": "test update",
                    "cadence": "manual"
                }),
                &lease(),
                &ctx,
            )
            .await
            .expect("create should succeed");
        assert!(!create_result.is_error);
        let mission_id = create_result
            .output
            .get("mission_id")
            .and_then(|v| v.as_str())
            .expect("should have mission_id");

        // Now update with string-typed guardrails — should fail.
        let update_result = adapter
            .execute_action(
                "mission_update",
                serde_json::json!({
                    "id": mission_id,
                    "max_concurrent": "5"
                }),
                &lease(),
                &ctx,
            )
            .await
            .expect("should return Ok with is_error=true");

        assert!(
            update_result.is_error,
            "string guardrails should fail: {}",
            update_result.output
        );
        assert!(
            update_result
                .output
                .get("error")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("must be an integer")),
            "got: {}",
            update_result.output
        );
    }

    /// Verify system_event cadence round-trips through mission_list.
    #[tokio::test]
    async fn system_event_cadence_round_trips_via_execute_action() {
        let adapter = make_adapter_with_missions().await;
        let ctx = exec_ctx(ironclaw_engine::ThreadId::new(), Some("rt1"));

        // Create a mission with a system_event cadence.
        let create_result = adapter
            .execute_action(
                "mission_create",
                serde_json::json!({
                    "name": "sys event test",
                    "goal": "test round-trip",
                    "cadence": "system_event:self-improvement/thread_completed"
                }),
                &lease(),
                &ctx,
            )
            .await
            .expect("create should succeed");
        assert!(
            !create_result.is_error,
            "create failed: {}",
            create_result.output
        );

        // List missions — the returned cadence should parse back.
        let list_result = adapter
            .execute_action("mission_list", serde_json::json!({}), &lease(), &ctx)
            .await
            .expect("list should succeed");
        assert!(!list_result.is_error);

        let missions = list_result.output.as_array().expect("should be array");
        let mission = missions
            .iter()
            .find(|m| m.get("name").and_then(|v| v.as_str()) == Some("sys event test"))
            .expect("should find the created mission");
        let cadence_str = mission
            .get("cadence")
            .and_then(|v| v.as_str())
            .expect("cadence should be a string");

        // The cadence string must parse back successfully.
        let round_tripped = parse_cadence(cadence_str, None);
        assert!(
            round_tripped.is_ok(),
            "cadence '{cadence_str}' failed to round-trip: {}",
            round_tripped.unwrap_err()
        );
    }

    /// Verify mission_complete returns "completed" status.
    #[tokio::test]
    async fn mission_complete_returns_completed_status_via_execute_action() {
        let adapter = make_adapter_with_missions().await;
        let ctx = exec_ctx(ironclaw_engine::ThreadId::new(), Some("d1"));

        let create_result = adapter
            .execute_action(
                "mission_create",
                serde_json::json!({
                    "name": "deletable",
                    "goal": "test delete",
                    "cadence": "manual"
                }),
                &lease(),
                &ctx,
            )
            .await
            .expect("create should succeed");
        assert!(!create_result.is_error);
        let mission_id = create_result
            .output
            .get("mission_id")
            .and_then(|v| v.as_str())
            .expect("should have mission_id");

        let delete_result = adapter
            .execute_action(
                "mission_complete",
                serde_json::json!({"id": mission_id}),
                &lease(),
                &ctx,
            )
            .await
            .expect("complete should succeed");

        assert!(!delete_result.is_error);
        assert_eq!(
            delete_result.output.get("status").and_then(|v| v.as_str()),
            Some("completed"),
            "mission_complete should return 'completed', got: {}",
            delete_result.output
        );
    }
}
