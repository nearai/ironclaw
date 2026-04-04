//! Policy enforcement engine — checks whether an agent action is permitted
//! under the configured policy thresholds.

use crate::config::PolicyConfig;
use crate::types::{AgentMessage, InferenceResponse, PolicyResult, PolicySeverity};

/// Policy engine that evaluates agent actions against configured thresholds.
#[derive(Debug)]
pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    /// Check if an agent message is allowed under the current policy.
    pub fn check(
        &self,
        message: &AgentMessage,
        _inference: &InferenceResponse,
    ) -> PolicyResult {
        let action = &message.payload.action;

        // ── Tool allowlist ────────────────────────────────────────────────
        if !self.config.allowed_tools.is_empty()
            && !self.config.allowed_tools.contains(action)
        {
            return PolicyResult {
                rule_id: "tool_allowlist".to_string(),
                severity: PolicySeverity::Block,
                details: format!("Tool '{action}' not in the agent's allowed list"),
            };
        }

        // ── Value threshold ────────────────────────────────────────────────
        if let Some(value) = message.payload.params.get("value") {
            if let Some(value_u64) = value.as_u64() {
                if value_u64 > self.config.max_value_autonomous_wei {
                    return PolicyResult {
                        rule_id: "value_threshold".to_string(),
                        severity: PolicySeverity::Warn,
                        details: format!(
                            "Action value {} wei exceeds autonomous threshold {} wei",
                            value_u64, self.config.max_value_autonomous_wei
                        ),
                    };
                }
            }
        }

        PolicyResult {
            rule_id: "default".to_string(),
            severity: PolicySeverity::Pass,
            details: "All policy checks passed".to_string(),
        }
    }

    /// Returns true if the given value requires Ledger approval.
    pub fn requires_ledger(&self, value_wei: u64) -> bool {
        value_wei > self.config.max_value_autonomous_wei
    }
}
