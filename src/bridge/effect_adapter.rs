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

use crate::bridge::auth_manager::{AuthCheckResult, AuthManager};
use crate::context::JobContext;
use crate::hooks::{HookEvent, HookOutcome, HookRegistry};
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
        }
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
                    credential_name: name.to_string(),
                    instructions: output_value
                        .get("instructions")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Complete authentication to continue.")
                        .to_string(),
                    auth_url: output_value
                        .get("auth_url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                },
                None,
            )),
            _ => None,
        }
    }

    /// Handle mission_* function calls. Returns None if not a mission call.
    async fn handle_mission_call(
        &self,
        action_name: &str,
        params: &serde_json::Value,
        context: &ThreadExecutionContext,
    ) -> Option<Result<ActionResult, EngineError>> {
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
                    .and_then(|v| v.as_str())
                    .unwrap_or("manual");
                // Use explicit timezone param, fall back to user's channel timezone.
                // ValidTimezone::parse filters empty/invalid strings.
                let timezone = params
                    .get("timezone")
                    .and_then(|v| v.as_str())
                    .and_then(ironclaw_engine::ValidTimezone::parse)
                    .or(context.user_timezone);
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
                match mgr
                    .create_mission(
                        context.project_id,
                        &context.user_id,
                        name,
                        goal,
                        parse_cadence(cadence_str, timezone),
                        notify_channels,
                    )
                    .await
                {
                    Ok(id) => {
                        Ok(serde_json::json!({"mission_id": id.to_string(), "status": "created"}))
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
                            serde_json::json!({
                                "id": m.id.to_string(),
                                "name": m.name,
                                "goal": m.goal,
                                "status": format!("{:?}", m.status),
                                "threads": m.thread_history.len(),
                                "current_focus": m.current_focus,
                                "notify_channels": m.notify_channels,
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
            "mission_delete" => {
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
                        Ok(()) => Ok(serde_json::json!({"status": "deleted"})),
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
                            updates.cadence = Some(parse_cadence(cadence, tz));
                        }
                        if let Some(arr) = params.get("notify_channels").and_then(|v| v.as_array())
                        {
                            updates.notify_channels = Some(
                                arr.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect(),
                            );
                        }
                        if let Some(max) =
                            params.get("max_threads_per_day").and_then(|v| v.as_u64())
                        {
                            updates.max_threads_per_day = Some(max as u32);
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

        Some(match result {
            Ok(output) => Ok(ActionResult {
                call_id: String::new(),
                action_name: action_name.to_string(),
                output,
                is_error: false,
                duration: std::time::Duration::ZERO,
            }),
            Err(e) => Ok(ActionResult {
                call_id: String::new(),
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
        let lookup_name = resolved_name.as_deref().unwrap_or(action_name);

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

        if let Some((_, tool)) = self.tools.get_resolved(action_name).await {
            // Defense-in-depth: reject V1Only tools even if they somehow got
            // a lease (e.g. via a stale capability registry or hallucination).
            if tool.engine_compatibility() == crate::tools::EngineCompatibility::V1Only {
                return Err(EngineError::Effect {
                    reason: format!(
                        "Tool '{}' is v1-only and not available in engine v2. \
                         Use the equivalent v2 workflow (e.g. mission_create instead of \
                         routine_create) or the appropriate slash command.",
                        action_name
                    ),
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
                        || self.auto_approved.read().await.contains(lookup_name);
                    if !is_approved && !approval_already_granted {
                        // Credential presence alone does NOT bypass approval.
                        // Credentials indicate the call *can* be authenticated,
                        // not that the user has authorized this specific request.
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

        if let Some(tool) = self.tools.get(lookup_name).await
            && let Some(rl_config) = tool.rate_limit_config()
        {
            let result = self
                .rate_limiter
                .check_and_record(&context.user_id, lookup_name, &rl_config)
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
                .check_action_auth(lookup_name, &parameters, &context.user_id, registry)
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
                            auth_url: cred.auth_url.clone(),
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

        let redacted_params = if let Some(tool) = self.tools.get(lookup_name).await {
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

        let job_ctx = JobContext::with_user(
            &context.user_id,
            "engine_v2",
            format!("Thread {}", context.thread_id),
        );

        let result = crate::tools::execute::execute_tool_with_safety(
            &self.tools,
            &self.safety,
            lookup_name,
            parameters.clone(),
            &job_ctx,
        )
        .await;

        let duration = start.elapsed();

        match result {
            Ok(output) => {
                let sanitized = self.safety.sanitize_tool_output(lookup_name, &output);
                let wrapped = self.safety.wrap_for_llm(lookup_name, &sanitized.content);
                let output_value = serde_json::from_str::<serde_json::Value>(&output)
                    .unwrap_or(serde_json::Value::String(wrapped));

                if (lookup_name == "tool_activate" || lookup_name == "tool_auth")
                    && let Some(err) = Self::auth_gate_from_extension_result(
                        action_name,
                        parameters.clone(),
                        context,
                        &output_value,
                    )
                {
                    return Err(err);
                }

                if (lookup_name == "tool_install" || lookup_name == "tool-install")
                    && let Some(auth_mgr) = self.auth_manager.read().await.as_ref()
                    && let Some(ext_name) = output_value.get("name").and_then(|v| v.as_str())
                {
                    use crate::bridge::auth_manager::ToolReadiness;
                    match auth_mgr
                        .check_tool_readiness(ext_name, &context.user_id)
                        .await
                    {
                        ToolReadiness::NeedsAuth {
                            auth_url,
                            instructions,
                            credential_name,
                        } => {
                            debug!(
                                extension = %ext_name,
                                credential = %credential_name,
                                "Post-install: extension needs auth — entering auth flow"
                            );
                            return Err(Self::gate_paused(
                                "authentication",
                                action_name,
                                context.current_call_id.as_deref(),
                                parameters,
                                ironclaw_engine::ResumeKind::Authentication {
                                    credential_name: credential_name.clone(),
                                    instructions: instructions.unwrap_or_else(|| {
                                        auth_mgr.get_setup_instructions_or_default(&credential_name)
                                    }),
                                    auth_url,
                                },
                                Some(output_value),
                            ));
                        }
                        ToolReadiness::NeedsSetup { ref message } => {
                            debug!(
                                extension = %ext_name,
                                "Post-install: extension needs setup"
                            );
                            let mut enriched = output_value.clone();
                            if let Some(obj) = enriched.as_object_mut() {
                                obj.insert(
                                    "auth_status".to_string(),
                                    serde_json::json!("needs_setup"),
                                );
                                obj.insert(
                                    "setup_message".to_string(),
                                    serde_json::Value::String(message.clone()),
                                );
                            }
                            return Ok(ActionResult {
                                call_id: String::new(),
                                action_name: action_name.to_string(),
                                output: enriched,
                                is_error: false,
                                duration,
                            });
                        }
                        ToolReadiness::Ready => {
                            debug!(
                                extension = %ext_name,
                                "Post-install: extension ready — no auth needed"
                            );
                        }
                    }
                }

                Ok(ActionResult {
                    call_id: String::new(),
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

                let sanitized = self.safety.sanitize_tool_output(lookup_name, &error_msg);

                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: action_name.to_string(),
                    output: serde_json::json!({"error": sanitized.content}),
                    is_error: true,
                    duration,
                })
            }
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

        let actions = tool_defs
            .into_iter()
            .map(|td| {
                let python_name = td.name.replace('-', "_");
                ActionDef {
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
                }
            })
            .collect();

        Ok(actions)
    }
}

/// Parse a cadence string into a MissionCadence.
///
/// When cadence is a cron expression, `timezone` is used as the scheduling
/// timezone. This is typically the user's channel timezone, auto-injected
/// from `ThreadExecutionContext::user_timezone`.
fn parse_cadence(
    s: &str,
    timezone: Option<ironclaw_engine::ValidTimezone>,
) -> ironclaw_engine::types::mission::MissionCadence {
    use ironclaw_engine::types::mission::MissionCadence;
    let trimmed = s.trim().to_lowercase();
    if trimmed == "manual" {
        MissionCadence::Manual
    } else if trimmed.contains(' ') && trimmed.split_whitespace().count() >= 5 {
        // Looks like a cron expression
        MissionCadence::Cron {
            expression: s.trim().to_string(),
            timezone,
        }
    } else if trimmed.starts_with("event:") {
        MissionCadence::OnEvent {
            event_pattern: trimmed
                .strip_prefix("event:")
                .unwrap_or("")
                .trim()
                .to_string(),
        }
    } else if trimmed.starts_with("webhook:") {
        MissionCadence::Webhook {
            path: trimmed
                .strip_prefix("webhook:")
                .unwrap_or("")
                .trim()
                .to_string(),
            secret: None,
        }
    } else {
        // Default to manual if unrecognized
        MissionCadence::Manual
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
            tools,
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
            tools,
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 10_000,
                injection_check_enabled: false,
            })),
            Arc::new(HookRegistry::default()),
        );

        // Set auth manager
        let auth_mgr = Arc::new(AuthManager::new(secrets, None, None));
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

        // Approval runs before auth in the current adapter pipeline, so a
        // missing-credential HTTP call that also needs approval pauses on the
        // approval gate first.
        match result {
            Err(EngineError::GatePaused { resume_kind, .. }) => match *resume_kind {
                ironclaw_engine::ResumeKind::Approval { allow_always } => {
                    assert!(allow_always);
                }
                other => panic!("Expected Approval gate, got: {other:?}"),
            },
            other => {
                panic!("Expected GatePaused for approval preflight, got: {other:?}");
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
}
