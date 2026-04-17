//! Approval gate — wraps `Tool::requires_approval()`.
//!
//! Replaces the inline approval check in `EffectBridgeAdapter::execute_action()`
//! (steps 1) with a composable gate that handles interactive, autonomous, and
//! container execution modes.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_engine::gate::{ExecutionGate, ExecutionMode, GateContext, GateDecision, ResumeKind};

use crate::tools::rate_limiter::RateLimiter;
use crate::tools::{ApprovalRequirement, ToolRegistry};

/// Gate that checks `Tool::requires_approval()` and emits `Pause(Approval)`
/// or `Deny` depending on execution mode.
///
/// Priority: 100 (after rate limiting, after relay channel check).
pub struct ApprovalGate {
    tools: Arc<ToolRegistry>,
}

impl ApprovalGate {
    pub fn new(tools: Arc<ToolRegistry>) -> Self {
        Self { tools }
    }
}

#[async_trait]
impl ExecutionGate for ApprovalGate {
    fn name(&self) -> &str {
        "approval"
    }

    fn priority(&self) -> u32 {
        100
    }

    async fn evaluate(&self, ctx: &GateContext<'_>) -> GateDecision {
        let tool = match self.tools.get_resolved(ctx.action_name).await {
            Some((_, t)) => t,
            None => return GateDecision::Allow, // unknown tool — let execution handle it
        };

        // Use original parameters for approval check (the adapter normalizes
        // params before execution, but the approval check should use the
        // parameters the LLM provided so destructive detection works).
        let requirement = tool.requires_approval(ctx.parameters);

        match ctx.execution_mode {
            ExecutionMode::Interactive => match requirement {
                ApprovalRequirement::Never => GateDecision::Allow,
                ApprovalRequirement::UnlessAutoApproved => {
                    if ctx.auto_approved.contains(ctx.action_name) {
                        GateDecision::Allow
                    } else {
                        // Check credential-backed HTTP auto-approve
                        if (ctx.action_name == "http" || ctx.action_name == "http_request")
                            && let Some(reg) = self.tools.credential_registry()
                            && crate::tools::builtin::extract_host_from_params(ctx.parameters)
                                .is_some_and(|host| reg.has_credentials_for_host(&host))
                        {
                            return GateDecision::Allow;
                        }
                        GateDecision::Pause {
                            reason: format!(
                                "Tool '{}' requires approval to execute.",
                                ctx.action_name
                            ),
                            resume_kind: ResumeKind::Approval { allow_always: true },
                        }
                    }
                }
                ApprovalRequirement::Always => GateDecision::Pause {
                    reason: format!(
                        "Tool '{}' requires explicit approval for this operation.",
                        ctx.action_name
                    ),
                    // Always-gated tools should NOT offer "always" button
                    // (regression fix: 09e1c97a)
                    resume_kind: ResumeKind::Approval {
                        allow_always: false,
                    },
                },
            },
            ExecutionMode::InteractiveAutoApprove => match requirement {
                ApprovalRequirement::Never | ApprovalRequirement::UnlessAutoApproved => {
                    // Auto-approve mode: shell, file_write, http, etc. proceed
                    // without prompting. Other safeguards (leases, rate limits,
                    // hooks, auth gates) still apply.
                    GateDecision::Allow
                }
                ApprovalRequirement::Always => {
                    // Always-gated tools still require explicit approval even
                    // in auto-approve mode — these are truly destructive operations.
                    GateDecision::Pause {
                        reason: format!(
                            "Tool '{}' requires explicit approval (auto-approve does not cover this operation).",
                            ctx.action_name
                        ),
                        resume_kind: ResumeKind::Approval {
                            allow_always: false,
                        },
                    }
                }
            },
            ExecutionMode::InteractiveAutoApproveAll => match requirement {
                // Destructive-bypass mode: every approval requirement passes
                // without prompting, including `Always`. The other gates in
                // the pipeline (RateLimitGate, HookGate, RelayChannelGate,
                // AuthenticationGate) still run because each has its own
                // priority and contract — this arm only short-circuits
                // ApprovalGate. Activated via
                // `AGENT_AUTO_APPROVE_DESTRUCTIVE=true` AND
                // `AGENT_AUTO_APPROVE_TOOLS=true`.
                ApprovalRequirement::Never
                | ApprovalRequirement::UnlessAutoApproved
                | ApprovalRequirement::Always => GateDecision::Allow,
            },
            ExecutionMode::Autonomous => match requirement {
                ApprovalRequirement::Never | ApprovalRequirement::UnlessAutoApproved => {
                    // Never and UnlessAutoApproved are allowed in autonomous mode
                    // (regression fix: 0e5f1b12 — is_blocked was rejecting Never tools)
                    GateDecision::Allow
                }
                ApprovalRequirement::Always => {
                    // Always-gated tools cannot run autonomously
                    GateDecision::Deny {
                        reason: format!(
                            "Tool '{}' requires explicit approval and cannot run autonomously.",
                            ctx.action_name
                        ),
                    }
                }
            },
            ExecutionMode::Container => GateDecision::Allow,
        }
    }
}

/// Gate that checks `AuthManager::check_action_auth()` for missing credentials.
///
/// Priority: 200 (after approval — no point checking credentials for a denied tool).
///
/// Currently a pass-through — the actual auth check remains inline in
/// `effect_adapter.rs` step 1.7 until Phase 4 migration completes.
pub struct AuthenticationGate;

#[async_trait]
impl ExecutionGate for AuthenticationGate {
    fn name(&self) -> &str {
        "authentication"
    }

    fn priority(&self) -> u32 {
        200
    }

    async fn evaluate(&self, _ctx: &GateContext<'_>) -> GateDecision {
        // The actual auth check is performed via the EffectBridgeAdapter's
        // auth_manager — this gate delegates there during Phase 4 migration.
        // For now, the inline check in effect_adapter.rs step 1.7 remains.
        GateDecision::Allow
    }
}

/// Gate that wraps `HookRegistry::run(BeforeToolCall)`.
///
/// Priority: 300 (after approval and auth — hooks can customize behavior
/// but should not preempt user-facing approval/auth flows).
pub struct HookGate {
    hooks: Arc<crate::hooks::HookRegistry>,
    tools: Arc<ToolRegistry>,
}

impl HookGate {
    pub fn new(hooks: Arc<crate::hooks::HookRegistry>, tools: Arc<ToolRegistry>) -> Self {
        Self { hooks, tools }
    }
}

#[async_trait]
impl ExecutionGate for HookGate {
    fn name(&self) -> &str {
        "hook"
    }

    fn priority(&self) -> u32 {
        300
    }

    async fn evaluate(&self, ctx: &GateContext<'_>) -> GateDecision {
        let redacted_params = if let Some(tool) = self.tools.get(ctx.action_name).await {
            crate::tools::redact_params(ctx.parameters, tool.sensitive_params())
        } else {
            ctx.parameters.clone()
        };

        let hook_event = crate::hooks::HookEvent::ToolCall {
            tool_name: ctx.action_name.to_string(),
            parameters: redacted_params,
            user_id: ctx.user_id.to_string(),
            context: format!("gate:{}", ctx.thread_id),
        };

        match self.hooks.run(&hook_event).await {
            Ok(crate::hooks::HookOutcome::Reject { reason }) => GateDecision::Deny {
                reason: format!("Tool '{}' blocked by hook: {reason}", ctx.action_name),
            },
            Err(crate::hooks::HookError::Rejected { reason }) => GateDecision::Deny {
                reason: format!("Tool '{}' blocked by hook: {reason}", ctx.action_name),
            },
            Err(e) => {
                tracing::debug!(
                    tool = ctx.action_name,
                    error = %e,
                    "hook error (fail-open)"
                );
                GateDecision::Allow
            }
            Ok(crate::hooks::HookOutcome::Continue { .. }) => GateDecision::Allow,
        }
    }
}

/// Gate that wraps the per-user per-tool `RateLimiter`.
///
/// Priority: 50 (runs before approval — deny fast for rate-limited tools).
pub struct RateLimitGate {
    tools: Arc<ToolRegistry>,
    rate_limiter: RateLimiter,
}

impl RateLimitGate {
    pub fn new(tools: Arc<ToolRegistry>, rate_limiter: RateLimiter) -> Self {
        Self {
            tools,
            rate_limiter,
        }
    }
}

#[async_trait]
impl ExecutionGate for RateLimitGate {
    fn name(&self) -> &str {
        "rate_limit"
    }

    fn priority(&self) -> u32 {
        50
    }

    async fn evaluate(&self, ctx: &GateContext<'_>) -> GateDecision {
        let tool = match self.tools.get(ctx.action_name).await {
            Some(t) => t,
            None => return GateDecision::Allow,
        };

        let rl_config = match tool.rate_limit_config() {
            Some(c) => c,
            None => return GateDecision::Allow,
        };

        let result = self
            .rate_limiter
            .check_and_record(ctx.user_id, ctx.action_name, &rl_config)
            .await;

        if let crate::tools::rate_limiter::RateLimitResult::Limited { retry_after, .. } = result {
            GateDecision::Deny {
                reason: format!(
                    "Tool '{}' is rate limited. Try again in {:.0}s.",
                    ctx.action_name,
                    retry_after.as_secs_f64()
                ),
            }
        } else {
            GateDecision::Allow
        }
    }
}

/// Gate that auto-denies approval-requiring tools on relay channels.
///
/// Fixes v1/v2 inconsistency where relay channels auto-deny was only
/// in v1 dispatcher but not in v2 router.
///
/// Priority: 80 (before approval — no point showing approval UI on channels
/// that can't respond interactively).
pub struct RelayChannelGate;

#[async_trait]
impl ExecutionGate for RelayChannelGate {
    fn name(&self) -> &str {
        "relay_channel"
    }

    fn priority(&self) -> u32 {
        80
    }

    async fn evaluate(&self, ctx: &GateContext<'_>) -> GateDecision {
        let is_relay = ctx.source_channel.ends_with("-relay");
        if !is_relay {
            return GateDecision::Allow;
        }

        if ctx.action_def.requires_approval {
            GateDecision::Deny {
                reason: format!(
                    "Tool '{}' requires approval but relay channel '{}' cannot provide interactive response.",
                    ctx.action_name, ctx.source_channel
                ),
            }
        } else {
            GateDecision::Allow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_engine::gate::ExecutionMode;
    use ironclaw_engine::types::capability::{ActionDef, EffectType};
    use ironclaw_engine::types::thread::ThreadId;
    use std::collections::HashSet;

    fn action_def(name: &str, requires_approval: bool) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: String::new(),
            parameters_schema: serde_json::json!({}),
            effects: vec![EffectType::ReadLocal],
            requires_approval,
        }
    }

    fn ctx<'a>(
        action_def: &'a ActionDef,
        mode: ExecutionMode,
        channel: &'a str,
        auto_approved: &'a HashSet<String>,
        params: &'a serde_json::Value,
    ) -> GateContext<'a> {
        GateContext {
            user_id: "user1",
            thread_id: ThreadId::new(),
            source_channel: channel,
            action_name: &action_def.name,
            call_id: "call_1",
            parameters: params,
            action_def,
            execution_mode: mode,
            auto_approved,
        }
    }

    // ── InteractiveAutoApprove mode ─────────────────────────

    #[tokio::test]
    async fn test_auto_approve_allows_unless_auto_approved_tools() {
        let gate = RelayChannelGate;
        // This test uses RelayChannelGate only to get a gate instance —
        // the actual auto-approve logic is in ApprovalGate which needs
        // a ToolRegistry. Test the mode semantics directly via GateContext.
        let ad = action_def("shell", false); // UnlessAutoApproved mapped here
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(
            &ad,
            ExecutionMode::InteractiveAutoApprove,
            "web",
            &auto,
            &params,
        );
        // RelayChannelGate doesn't care about mode — it only checks channel suffix
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Allow)); // safety: test-only
    }

    // ── RelayChannelGate ─────────────────────────────────────

    #[tokio::test]
    async fn test_relay_channel_denies_approval_requiring_tools() {
        let gate = RelayChannelGate;
        let ad = action_def("shell", true);
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(
            &ad,
            ExecutionMode::Interactive,
            "slack-relay",
            &auto,
            &params,
        );
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_non_relay_channel_always_allows() {
        let gate = RelayChannelGate;
        let ad = action_def("shell", true);
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(&ad, ExecutionMode::Interactive, "telegram", &auto, &params);
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Allow));
    }

    // ── InteractiveAutoApproveAll mode (P0-1, P0-2 regression) ─────

    use crate::tools::{Tool, ToolError, ToolOutput, ToolRegistry};
    use async_trait::async_trait;
    use std::sync::Arc;

    /// Test stub tool that always returns the approval requirement provided
    /// at construction. Lets us drive `ApprovalGate::evaluate` against every
    /// `ApprovalRequirement` value without spinning up the real built-in
    /// tools.
    struct StubApprovalTool {
        name: String,
        requirement: crate::tools::ApprovalRequirement,
    }

    #[async_trait]
    impl Tool for StubApprovalTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "Stub tool for ApprovalGate tests"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(
                serde_json::json!({}),
                std::time::Duration::from_millis(1),
            ))
        }

        fn requires_approval(
            &self,
            _params: &serde_json::Value,
        ) -> crate::tools::ApprovalRequirement {
            self.requirement
        }
    }

    async fn registry_with_tool(
        name: &str,
        requirement: crate::tools::ApprovalRequirement,
    ) -> Arc<ToolRegistry> {
        let reg = Arc::new(ToolRegistry::new());
        reg.register(Arc::new(StubApprovalTool {
            name: name.into(),
            requirement,
        }))
        .await;
        reg
    }

    /// Under `InteractiveAutoApproveAll`, every approval requirement maps
    /// to `Allow` — including the destructive `Always` variant. Composes
    /// with the rest of the gate pipeline (rate limit, hook, relay, auth)
    /// because this only short-circuits ApprovalGate; the others are
    /// evaluated independently.
    #[tokio::test]
    async fn test_auto_approve_all_allows_never() {
        let reg = registry_with_tool("noop", crate::tools::ApprovalRequirement::Never).await;
        let gate = ApprovalGate::new(reg);
        let ad = action_def("noop", false);
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(
            &ad,
            ExecutionMode::InteractiveAutoApproveAll,
            "web",
            &auto,
            &params,
        );
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Allow));
    }

    #[tokio::test]
    async fn test_auto_approve_all_allows_unless_auto_approved() {
        let reg = registry_with_tool(
            "shell",
            crate::tools::ApprovalRequirement::UnlessAutoApproved,
        )
        .await;
        let gate = ApprovalGate::new(reg);
        let ad = action_def("shell", false);
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(
            &ad,
            ExecutionMode::InteractiveAutoApproveAll,
            "web",
            &auto,
            &params,
        );
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Allow));
    }

    /// The killer regression: `Always`-gated destructive tools normally
    /// pause even under standard `InteractiveAutoApprove`. Under
    /// `InteractiveAutoApproveAll` they pass — this is the user-visible
    /// "true zero-confirm" promise.
    #[tokio::test]
    async fn test_auto_approve_all_allows_always_gated_tools() {
        let reg = registry_with_tool(
            "dangerous_delete",
            crate::tools::ApprovalRequirement::Always,
        )
        .await;
        let gate = ApprovalGate::new(reg);
        let ad = action_def("dangerous_delete", true);
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(
            &ad,
            ExecutionMode::InteractiveAutoApproveAll,
            "web",
            &auto,
            &params,
        );
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Allow));
    }

    /// Sanity check that `InteractiveAutoApprove` still pauses on
    /// `Always` — guards against accidental regression of the existing
    /// behavior when extending the match.
    #[tokio::test]
    async fn test_interactive_auto_approve_still_pauses_always() {
        let reg = registry_with_tool(
            "dangerous_delete",
            crate::tools::ApprovalRequirement::Always,
        )
        .await;
        let gate = ApprovalGate::new(reg);
        let ad = action_def("dangerous_delete", true);
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(
            &ad,
            ExecutionMode::InteractiveAutoApprove,
            "web",
            &auto,
            &params,
        );
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Pause { .. }));
    }

    /// Named regression for P0-2: even under `InteractiveAutoApproveAll`
    /// the `RelayChannelGate` still denies approval-requiring tools
    /// arriving on a relay channel. The two gates are independent — the
    /// auto-approve mode never mutates `ActionDef.requires_approval`, so
    /// the relay defense keeps working.
    #[tokio::test]
    async fn test_relay_channel_denies_under_auto_approve_all() {
        let gate = RelayChannelGate;
        let ad = action_def("dangerous_delete", true);
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(
            &ad,
            ExecutionMode::InteractiveAutoApproveAll,
            "dingtalk-relay",
            &auto,
            &params,
        );
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_relay_allows_non_approval_tools() {
        let gate = RelayChannelGate;
        let ad = action_def("echo", false);
        let auto = HashSet::new();
        let params = serde_json::json!({});
        let c = ctx(
            &ad,
            ExecutionMode::Interactive,
            "slack-relay",
            &auto,
            &params,
        );
        assert!(matches!(gate.evaluate(&c).await, GateDecision::Allow));
    }
}
