//! LLM-as-Judge hook: semantically evaluates tool calls for intent alignment.
//!
//! Registered as a [`HookPoint::BeforeToolCall`] hook when
//! `SAFETY_LLM_JUDGE_ENABLED=true`. Runs AFTER heuristic safety checks and
//! BEFORE tool execution. Disabled by default — zero overhead when off.
//!
//! On approval-resumed calls the `intent` field is `""` — the hook skips
//! evaluation because the user already explicitly authorised the tool.

use std::sync::Arc;

use async_trait::async_trait;

use crate::hooks::{Hook, HookContext, HookError, HookEvent, HookOutcome, HookPoint};
use crate::safety::{AmbiguousPolicy, JudgeVerdict, LlmJudge, ToolCallRequest};

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

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let HookEvent::ToolCall {
            tool_name,
            parameters,
            intent,
            ..
        } = event
        else {
            return Ok(HookOutcome::ok());
        };

        // Skip when intent is empty (approval-resumed calls where user already authorised).
        if intent.is_empty() {
            return Ok(HookOutcome::ok());
        }

        let req = ToolCallRequest {
            tool_name: tool_name.clone(),
            tool_args: parameters.clone(),
            original_user_intent: intent.clone(),
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
