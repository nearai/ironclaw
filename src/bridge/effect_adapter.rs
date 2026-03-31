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

/// Callback invoked when a credential is missing and the user needs to authenticate.
/// Parameters: (credential_name, action_name).
/// The router sets this to emit SSE events; mission threads may have a no-op.
pub type AuthRequiredCallback = Box<dyn Fn(&str, &str) + Send + Sync>;

/// Wraps the existing tool pipeline to implement the engine's `EffectExecutor`.
///
/// Enforces all v1 security controls at the adapter boundary:
/// tool approval, output sanitization, hooks, rate limiting, and call limits.
pub struct EffectBridgeAdapter {
    tools: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
    hooks: Arc<HookRegistry>,
    /// Tools the user has approved with "always" (persists within session).
    auto_approved: RwLock<HashSet<String>>,
    /// Per-step tool call counter (reset externally between steps).
    call_count: std::sync::atomic::AtomicU32,
    /// Per-user per-tool sliding window rate limiter.
    rate_limiter: RateLimiter,
    /// Mission manager for handling mission_* function calls.
    mission_manager: RwLock<Option<Arc<ironclaw_engine::MissionManager>>>,
    /// Optional callback for when a credential is missing (emits AuthRequired SSE).
    auth_required_callback: RwLock<Option<Arc<AuthRequiredCallback>>>,
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
            auto_approved: RwLock::new(HashSet::new()),
            call_count: std::sync::atomic::AtomicU32::new(0),
            rate_limiter: RateLimiter::new(),
            mission_manager: RwLock::new(None),
            auth_required_callback: RwLock::new(None),
            auth_manager: RwLock::new(None),
        }
    }

    /// Set the callback invoked when a credential is missing.
    pub async fn set_auth_required_callback(&self, cb: Arc<AuthRequiredCallback>) {
        *self.auth_required_callback.write().await = Some(cb);
    }

    /// Emit an auth_required signal (best-effort, non-blocking).
    async fn emit_auth_required(&self, credential_name: &str, action_name: &str) {
        if let Some(cb) = self.auth_required_callback.read().await.as_ref() {
            cb(credential_name, action_name);
        }
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
                match mgr
                    .create_mission(
                        context.project_id,
                        &context.user_id,
                        name,
                        goal,
                        parse_cadence(cadence_str),
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
}

#[async_trait::async_trait]
impl EffectExecutor for EffectBridgeAdapter {
    async fn execute_action(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        _lease: &CapabilityLease,
        context: &ThreadExecutionContext,
    ) -> Result<ActionResult, EngineError> {
        let start = Instant::now();

        // Resolve tool name (underscore → hyphen fallback)
        let hyphenated = action_name.replace('_', "-");
        let lookup_name = if self.tools.get(action_name).await.is_some() {
            action_name
        } else {
            &hyphenated
        };

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

        // ── 0a. Handle mission_* functions via MissionManager ──
        if let Some(result) = self
            .handle_mission_call(action_name, &parameters, context)
            .await
        {
            return result.map(|mut r| {
                r.duration = start.elapsed();
                r
            });
        }

        // ── 0b. Block tools that need v1 runtime deps (RoutineEngine, Scheduler) ──
        if is_v1_only_tool(lookup_name) {
            return Err(EngineError::Effect {
                reason: format!(
                    "Tool '{}' is not available in engine v2. \
                     Tell the user to use the slash command instead (e.g. /routine, /job).",
                    action_name
                ),
            });
        }

        // ── 0c. Block v1 auth management tools (auth is kernel-level in v2) ──
        if is_v1_auth_tool(lookup_name) {
            return Err(EngineError::Effect {
                reason: format!(
                    "Tool '{}' is not available in engine v2. \
                     Authentication is handled automatically by the kernel.",
                    action_name
                ),
            });
        }

        // ── 1. Check tool approval (v1: Tool::requires_approval) ──

        if let Some(tool) = self.tools.get(lookup_name).await {
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
                    let is_approved = self.auto_approved.read().await.contains(lookup_name);
                    if !is_approved {
                        // In v2, credential-backed HTTP calls are auto-approved.
                        // The user authorized by storing the credential — the v1
                        // interactive approval flow doesn't exist in v2.
                        let has_credential_backing = lookup_name == "http"
                            && self.tools.credential_registry().is_some_and(|reg| {
                                crate::tools::builtin::extract_host_from_params(&parameters)
                                    .is_some_and(|host| reg.has_credentials_for_host(&host))
                            });

                        if !has_credential_backing {
                            return Err(EngineError::NeedApproval {
                                action_name: action_name.to_string(),
                                call_id: String::new(),
                                parameters,
                            });
                        }
                    }
                }
                ApprovalRequirement::Never => {}
            }
        }

        // ── 1.5. Check rate limit (v1: RateLimiter) ──

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

        // ── 1.7. Pre-flight auth check (credential gate) ──
        //
        // Before executing any tool, check if required credentials exist.
        // If missing, return NeedAuthentication immediately — the tool never
        // executes and the user never sees a 401. This is the primary auth
        // interception point; post-execution 401 detection and text-based
        // fallback remain as defense-in-depth.

        {
            let has_mgr = self.auth_manager.read().await.is_some();
            let has_reg = self.tools.credential_registry().is_some();
            // Use warn! so this is visible in E2E test logs (debug may be filtered)
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
                    self.emit_auth_required(&cred.credential_name, action_name)
                        .await;
                    return Err(EngineError::NeedAuthentication {
                        credential_name: cred.credential_name.clone(),
                        action_name: action_name.to_string(),
                        call_id: String::new(),
                        parameters,
                    });
                }
                AuthCheckResult::Ready => {
                    debug!(tool = %lookup_name, "Pre-flight auth: credentials present");
                }
                AuthCheckResult::NoAuthRequired => {}
            }
        }

        // ── 2. Run BeforeToolCall hook (v1: hooks.run) ──

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

        // ── 3. Execute through existing safety pipeline ──

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

        // ── 4. Sanitize + wrap output (v1: sanitize_tool_output + wrap_for_llm) ──

        match result {
            Ok(output) => {
                // Apply v1 sanitization: leak detection, policy, truncation
                let sanitized = self.safety.sanitize_tool_output(lookup_name, &output);

                // Wrap for LLM: XML boundary protection against injection
                let wrapped = self.safety.wrap_for_llm(lookup_name, &sanitized.content);

                // Parse wrapped content as JSON if possible (for Python dict access)
                // But keep the safety wrapping in the raw output
                let output_value = serde_json::from_str::<serde_json::Value>(&output)
                    .unwrap_or(serde_json::Value::String(wrapped));

                // ── 4a. Post-install auth pipeline ──
                //
                // After tool_install succeeds, check whether the installed
                // extension needs credentials. If so, return NeedAuthentication
                // to trigger the auth flow. The router stores the original
                // message and retries after credentials are provided.
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
                            credential_name, ..
                        } => {
                            debug!(
                                extension = %ext_name,
                                credential = %credential_name,
                                "Post-install: extension needs auth — entering auth flow"
                            );
                            self.emit_auth_required(&credential_name, ext_name).await;
                            return Err(EngineError::NeedAuthentication {
                                credential_name,
                                action_name: action_name.to_string(),
                                call_id: String::new(),
                                parameters,
                            });
                        }
                        ToolReadiness::NeedsSetup { ref message } => {
                            // Can't auto-resolve — append setup info to install result.
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

                // Detect authentication_required errors from the HTTP tool.
                // Return NeedAuthentication so the engine pauses the thread
                // and the router can prompt the user for credentials.
                if error_msg.contains("authentication_required")
                    && let Some(cred_name) = extract_credential_name(&error_msg)
                {
                    tracing::warn!(
                        credential = %cred_name,
                        tool = %lookup_name,
                        user = %context.user_id,
                        "Credential missing — returning NeedAuthentication"
                    );
                    self.emit_auth_required(&cred_name, action_name).await;
                    return Err(EngineError::NeedAuthentication {
                        credential_name: cred_name,
                        action_name: action_name.to_string(),
                        call_id: String::new(),
                        parameters,
                    });
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

            // Check default approval requirement (with empty params)
            let requires_approval = if let Some(tool) = self.tools.get(&td.name).await {
                !matches!(
                    tool.requires_approval(&serde_json::json!({})),
                    ApprovalRequirement::Never
                )
            } else {
                false
            };

            actions.push(ActionDef {
                name: python_name,
                description: td.description,
                parameters_schema: td.parameters,
                effects: vec![],
                requires_approval,
            });
        }

        Ok(actions)
    }
}

/// Parse a cadence string into a MissionCadence.
fn parse_cadence(s: &str) -> ironclaw_engine::types::mission::MissionCadence {
    use ironclaw_engine::types::mission::MissionCadence;
    let trimmed = s.trim().to_lowercase();
    if trimmed == "manual" {
        MissionCadence::Manual
    } else if trimmed.contains(' ') && trimmed.split_whitespace().count() >= 5 {
        // Looks like a cron expression
        MissionCadence::Cron {
            expression: s.trim().to_string(),
            timezone: None,
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

fn is_v1_only_tool(name: &str) -> bool {
    matches!(
        name,
        "create_job"
            | "create-job"
            | "cancel_job"
            | "cancel-job"
            | "build_software"
            | "build-software"
            | "routine_create"
            | "routine_list"
            | "routine_fire"
            | "routine_pause"
            | "routine_resume"
            | "routine_update"
            | "routine_delete"
    )
}

/// Auth management tools from v1 that are now kernel-internal in v2.
/// The LLM should not see or call these — auth is handled automatically.
fn is_v1_auth_tool(name: &str) -> bool {
    matches!(
        name,
        "tool_auth" | "tool-auth" | "tool_activate" | "tool-activate"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── auth_required_callback tests ───────────────────────────

    #[tokio::test]
    async fn auth_callback_fires_on_missing_credential() {
        let adapter = make_adapter();

        let fired = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let fired_clone = Arc::clone(&fired);

        adapter
            .set_auth_required_callback(Arc::new(Box::new(move |cred, action| {
                fired_clone
                    .lock()
                    .unwrap()
                    .push((cred.to_string(), action.to_string()));
            })))
            .await;

        adapter.emit_auth_required("github_token", "http").await;

        let calls = fired.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "github_token");
        assert_eq!(calls[0].1, "http");
    }

    #[tokio::test]
    async fn auth_callback_not_set_is_noop() {
        let adapter = make_adapter();
        // No callback set — should not panic
        adapter.emit_auth_required("some_token", "http").await;
    }

    // ── is_v1_only_tool tests ──────────────────────────────────

    #[test]
    fn routine_tools_are_v1_only() {
        assert!(is_v1_only_tool("routine_create"));
        assert!(is_v1_only_tool("routine_list"));
        assert!(is_v1_only_tool("routine_fire"));
        assert!(is_v1_only_tool("routine_delete"));
        assert!(is_v1_only_tool("routine_pause"));
        assert!(is_v1_only_tool("routine_resume"));
        assert!(is_v1_only_tool("routine_update"));
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
        assert!(is_v1_auth_tool("tool_activate"));
        assert!(is_v1_auth_tool("tool-activate"));
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
            granted_actions: vec![],
            granted_at: chrono::Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
        };
        let ctx = ironclaw_engine::ThreadExecutionContext {
            thread_id: ironclaw_engine::ThreadId::new(),
            thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
            project_id: ironclaw_engine::ProjectId::new(),
            user_id: "test_user".to_string(),
            step_id: ironclaw_engine::StepId::new(),
        };

        let result = adapter.execute_action("http", params, &lease, &ctx).await;

        // Should return NeedAuthentication, NOT execute the tool
        match result {
            Err(EngineError::NeedAuthentication {
                credential_name, ..
            }) => {
                assert_eq!(credential_name, "github_token");
            }
            other => {
                panic!("Expected NeedAuthentication for missing github_token, got: {other:?}");
            }
        }
    }
}
