//! LLM-as-Judge hook: semantically evaluates tool calls for intent alignment.
//!
//! Registered as a [`HookPoint::BeforeToolCall`] hook when
//! `SAFETY_LLM_JUDGE_ENABLED=true`. Runs AFTER heuristic safety checks and
//! BEFORE tool execution. Disabled by default — zero overhead when off.
//!
//! On approval-resumed calls the `intent` field is `None` — the hook skips
//! evaluation because the user already explicitly authorised the tool.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use ironclaw_safety::{AmbiguousPolicy, JudgeVerdict, LlmJudge, ToolCallRequest};

use crate::hooks::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};

/// Hook that runs the LLM judge before every tool call.
pub struct LlmJudgeHook {
    judge: Arc<LlmJudge>,
}

impl LlmJudgeHook {
    pub fn new(judge: Arc<LlmJudge>) -> Self {
        Self { judge }
    }
}

#[async_trait]
impl Hook for LlmJudgeHook {
    fn name(&self) -> &str {
        "llm_judge"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &[HookPoint::BeforeToolCall]
    }

    /// Mirror the operator-configured judge timeout so the hook registry
    /// doesn't cap it at the default 5 s before the judge LLM has a chance
    /// to respond. Without this override, setting SAFETY_LLM_JUDGE_TIMEOUT_MS
    /// beyond 5000 has no effect — the registry fires first.
    fn timeout(&self) -> Duration {
        Duration::from_millis(self.judge.config.timeout_ms)
    }

    /// Fail-open: a judge timeout/outage must not brick the assistant.
    /// The judge is a second-opinion layer, not a hard gate; availability
    /// outages fall back to the heuristic safety layer that ran earlier.
    fn failure_mode(&self) -> HookFailureMode {
        HookFailureMode::FailOpen
    }

    async fn execute(
        &self,
        event: &HookEvent,
        ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let HookEvent::ToolCall {
            tool_name,
            parameters,
            ..
        } = event
        else {
            return Ok(HookOutcome::ok());
        };

        // Skip when intent is absent.
        // - Interactive chat (dispatcher.rs): always has intent.
        // - Autonomous job workers (job.rs): pass the job description as intent.
        // - Engine-v2 / gate paths: no user intent available, judge is skipped.
        // - Approval-resumed calls: the directly-approved call is skipped here;
        //   deferred siblings are re-evaluated by process_approval in thread_ops.
        let Some(intent) = ctx.intent.as_deref() else {
            return Ok(HookOutcome::ok());
        };

        let req = ToolCallRequest {
            tool_name: tool_name.clone(),
            tool_args: parameters.clone(),
            original_user_intent: intent.to_string(),
        };

        let (verdict, record) = self.judge.evaluate(&req).await;

        tracing::debug!(
            tool = %tool_name,
            verdict = %record.verdict,
            confidence = record.confidence,
            latency_ms = record.latency_ms,
            "LLM judge result"
        );

        match verdict {
            JudgeVerdict::Allow => Ok(HookOutcome::ok()),
            JudgeVerdict::Deny(reason) => {
                tracing::warn!(
                    tool = %tool_name,
                    reason = %reason,
                    attack_type = ?record.attack_type,
                    "LLM judge denied tool call"
                );
                Ok(HookOutcome::reject(format!(
                    "LLM judge denied tool call '{tool_name}': {reason}"
                )))
            }
            JudgeVerdict::Ambiguous(reason) => match self.judge.config.ambiguous_policy {
                AmbiguousPolicy::Block => {
                    tracing::warn!(
                        tool = %tool_name,
                        reason = %reason,
                        "LLM judge: ambiguous verdict blocked by policy"
                    );
                    Ok(HookOutcome::reject(format!(
                        "LLM judge: ambiguous verdict for '{tool_name}' blocked by policy: {reason}"
                    )))
                }
                AmbiguousPolicy::Allow => {
                    tracing::debug!(
                        tool = %tool_name,
                        reason = %reason,
                        "LLM judge: ambiguous verdict allowed by policy"
                    );
                    Ok(HookOutcome::ok())
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_safety::{JudgeLlm, LlmJudgeConfig};

    struct MockJudgeLlm(Result<String, String>);

    #[async_trait]
    impl JudgeLlm for MockJudgeLlm {
        async fn complete_text(
            &self,
            _system: &str,
            _user: &str,
            _model_override: Option<&str>,
            _max_tokens: u32,
        ) -> Result<String, String> {
            self.0.clone()
        }
    }

    fn make_judge(response: &str, ambiguous_policy: AmbiguousPolicy) -> Arc<LlmJudge> {
        let llm = Arc::new(MockJudgeLlm(Ok(response.to_string())));
        let config = LlmJudgeConfig {
            enabled: true,
            model: None,
            base_url: None,
            api_key: None,
            confidence_threshold: 0.70,
            ambiguous_policy,
            timeout_ms: 5_000,
        };
        Arc::new(LlmJudge::new(llm, config))
    }

    fn tool_call_event() -> HookEvent {
        HookEvent::ToolCall {
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"cmd": "ls"}),
            user_id: "u1".to_string(),
            context: "chat".to_string(),
        }
    }

    fn ctx_with_intent(intent: &str) -> HookContext {
        HookContext {
            intent: Some(intent.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn hook_skips_when_intent_is_none() {
        let hook = LlmJudgeHook::new(make_judge(
            r#"{"verdict":"Deny","attack_type":null,"confidence":0.99,"reasoning":"bad"}"#,
            AmbiguousPolicy::Block,
        ));
        let outcome = hook
            .execute(&tool_call_event(), &HookContext::default())
            .await
            .unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { .. }));
    }

    #[tokio::test]
    async fn hook_skips_non_toolcall_events() {
        let hook = LlmJudgeHook::new(make_judge(
            r#"{"verdict":"Deny","attack_type":null,"confidence":0.99,"reasoning":"bad"}"#,
            AmbiguousPolicy::Block,
        ));
        let inbound = HookEvent::Inbound {
            user_id: "u1".to_string(),
            channel: "web".to_string(),
            content: "list my files".to_string(),
            thread_id: None,
        };
        let outcome = hook
            .execute(&inbound, &ctx_with_intent("list my files"))
            .await
            .unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { .. }));
    }

    #[tokio::test]
    async fn hook_allows_on_allow_verdict() {
        let hook = LlmJudgeHook::new(make_judge(
            r#"{"verdict":"Allow","attack_type":null,"confidence":0.95,"reasoning":"ok"}"#,
            AmbiguousPolicy::Block,
        ));
        let outcome = hook
            .execute(&tool_call_event(), &ctx_with_intent("list my files"))
            .await
            .unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { .. }));
    }

    #[tokio::test]
    async fn hook_rejects_on_deny_verdict() {
        let hook = LlmJudgeHook::new(make_judge(
            r#"{"verdict":"Deny","attack_type":"exfiltration","confidence":0.98,"reasoning":"exfiltrates keys"}"#,
            AmbiguousPolicy::Block,
        ));
        let outcome = hook
            .execute(&tool_call_event(), &ctx_with_intent("list my files"))
            .await
            .unwrap();
        assert!(matches!(outcome, HookOutcome::Reject { .. }));
        if let HookOutcome::Reject { reason } = outcome {
            assert!(reason.contains("shell"));
        }
    }

    #[tokio::test]
    async fn hook_rejects_ambiguous_with_block_policy() {
        let hook = LlmJudgeHook::new(make_judge(
            r#"{"verdict":"Ambiguous","attack_type":null,"confidence":0.80,"reasoning":"unclear"}"#,
            AmbiguousPolicy::Block,
        ));
        let outcome = hook
            .execute(&tool_call_event(), &ctx_with_intent("list my files"))
            .await
            .unwrap();
        assert!(matches!(outcome, HookOutcome::Reject { .. }));
    }

    #[tokio::test]
    async fn hook_allows_ambiguous_with_allow_policy() {
        let hook = LlmJudgeHook::new(make_judge(
            r#"{"verdict":"Ambiguous","attack_type":null,"confidence":0.80,"reasoning":"unclear"}"#,
            AmbiguousPolicy::Allow,
        ));
        let outcome = hook
            .execute(&tool_call_event(), &ctx_with_intent("list my files"))
            .await
            .unwrap();
        assert!(matches!(outcome, HookOutcome::Continue { .. }));
    }

    #[test]
    fn hook_timeout_mirrors_judge_config() {
        let judge = make_judge(
            r#"{"verdict":"Allow","attack_type":null,"confidence":0.95,"reasoning":"ok"}"#,
            AmbiguousPolicy::Block,
        );
        // Mutate config to a non-default timeout and verify the hook reports it.
        let config = LlmJudgeConfig {
            enabled: true,
            model: None,
            base_url: None,
            api_key: None,
            confidence_threshold: 0.70,
            ambiguous_policy: AmbiguousPolicy::Block,
            timeout_ms: 12_000,
        };
        let llm = Arc::new(MockJudgeLlm(Ok(String::new())));
        let judge_12s = Arc::new(LlmJudge::new(llm, config));
        let hook = LlmJudgeHook::new(judge_12s);
        assert_eq!(hook.timeout(), Duration::from_millis(12_000));
        // Default judge (5 s) should also round-trip.
        let hook_default = LlmJudgeHook::new(judge);
        assert_eq!(hook_default.timeout(), Duration::from_millis(5_000));
    }
}
