//! Deterministic policy engine.
//!
//! Evaluates whether an action is allowed, denied, or requires approval
//! based on effect types, capability policies, and thread leases.
//! No LLM calls — purely deterministic.

use crate::types::capability::{
    ActionDef, CapabilityLease, EffectType, PolicyCondition, PolicyEffect, PolicyRule,
};

/// The result of a policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { reason: String },
}

/// Deterministic policy engine.
///
/// Evaluation precedence: Deny > RequireApproval > Allow.
/// Checks are evaluated in order: global policies, then capability policies,
/// then action-level `requires_approval`, then effect-type checks against
/// the lease's allowed effects.
pub struct PolicyEngine {
    global_policies: Vec<PolicyRule>,
    /// Effect types that are always denied unless explicitly overridden.
    denied_effects: Vec<EffectType>,
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self {
            global_policies: Vec::new(),
            denied_effects: Vec::new(),
        }
    }

    /// Add a global policy rule.
    pub fn add_global_policy(&mut self, rule: PolicyRule) {
        self.global_policies.push(rule);
    }

    /// Add an effect type that is always denied.
    pub fn deny_effect(&mut self, effect: EffectType) {
        self.denied_effects.push(effect);
    }

    /// Evaluate whether an action is allowed given a lease and capability policies.
    pub fn evaluate(
        &self,
        action: &ActionDef,
        lease: &CapabilityLease,
        capability_policies: &[PolicyRule],
    ) -> PolicyDecision {
        // 1. Check lease validity
        if !lease.is_valid() {
            return PolicyDecision::Deny {
                reason: format!("lease for {} is expired/revoked", lease.capability_name),
            };
        }

        // 2. Check lease covers this action
        if !lease.covers_action(&action.name) {
            return PolicyDecision::Deny {
                reason: format!(
                    "lease for {} does not cover action {}",
                    lease.capability_name, action.name
                ),
            };
        }

        // 3. Check denied effect types
        for effect in &action.effects {
            if self.denied_effects.contains(effect) {
                return PolicyDecision::Deny {
                    reason: format!("effect type {effect:?} is denied by global policy"),
                };
            }
        }

        // 4. Evaluate global policies
        let mut decision = PolicyDecision::Allow;
        for rule in &self.global_policies {
            if rule_matches(rule, action) {
                decision = merge_decision(decision, rule.effect, &rule.name);
            }
        }

        // 5. Evaluate capability-level policies
        for rule in capability_policies {
            if rule_matches(rule, action) {
                decision = merge_decision(decision, rule.effect, &rule.name);
            }
        }

        // 6. Check action-level requires_approval
        if action.requires_approval {
            decision = merge_decision(
                decision,
                PolicyEffect::RequireApproval,
                "action requires approval",
            );
        }

        decision
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Check whether a policy rule's condition matches the given action.
fn rule_matches(rule: &PolicyRule, action: &ActionDef) -> bool {
    match &rule.condition {
        PolicyCondition::Always => true,
        PolicyCondition::ActionMatches { pattern } => action.name.contains(pattern.as_str()),
        PolicyCondition::EffectTypeIs(effect) => action.effects.contains(effect),
    }
}

/// Merge a new policy effect into the current decision.
/// Deny > RequireApproval > Allow.
fn merge_decision(current: PolicyDecision, effect: PolicyEffect, source: &str) -> PolicyDecision {
    match effect {
        PolicyEffect::Deny => PolicyDecision::Deny {
            reason: source.to_string(),
        },
        PolicyEffect::RequireApproval => match current {
            PolicyDecision::Deny { .. } => current,
            _ => PolicyDecision::RequireApproval {
                reason: source.to_string(),
            },
        },
        PolicyEffect::Allow => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::LeaseId;
    use crate::types::thread::ThreadId;
    use chrono::Utc;

    fn make_action(name: &str, effects: Vec<EffectType>, requires_approval: bool) -> ActionDef {
        ActionDef {
            name: name.into(),
            description: String::new(),
            parameters_schema: serde_json::json!({}),
            effects,
            requires_approval,
        }
    }

    fn make_lease() -> CapabilityLease {
        CapabilityLease {
            id: LeaseId::new(),
            thread_id: ThreadId::new(),
            capability_name: "test".into(),
            granted_actions: vec![],
            granted_at: Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
        }
    }

    #[test]
    fn allow_by_default() {
        let engine = PolicyEngine::new();
        let action = make_action("read_file", vec![EffectType::ReadLocal], false);
        let lease = make_lease();
        assert_eq!(engine.evaluate(&action, &lease, &[]), PolicyDecision::Allow);
    }

    #[test]
    fn denied_effect_type() {
        let mut engine = PolicyEngine::new();
        engine.deny_effect(EffectType::Financial);
        let action = make_action("transfer", vec![EffectType::Financial], false);
        let lease = make_lease();
        assert!(matches!(
            engine.evaluate(&action, &lease, &[]),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn action_requires_approval() {
        let engine = PolicyEngine::new();
        let action = make_action("deploy", vec![EffectType::WriteExternal], true);
        let lease = make_lease();
        assert!(matches!(
            engine.evaluate(&action, &lease, &[]),
            PolicyDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn global_policy_deny_overrides_approval() {
        let mut engine = PolicyEngine::new();
        engine.add_global_policy(PolicyRule {
            name: "no external writes".into(),
            condition: PolicyCondition::EffectTypeIs(EffectType::WriteExternal),
            effect: PolicyEffect::Deny,
        });
        let action = make_action("deploy", vec![EffectType::WriteExternal], true);
        let lease = make_lease();
        assert!(matches!(
            engine.evaluate(&action, &lease, &[]),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn capability_policy_requires_approval() {
        let engine = PolicyEngine::new();
        let action = make_action("create_issue", vec![EffectType::WriteExternal], false);
        let lease = make_lease();
        let cap_policies = vec![PolicyRule {
            name: "approve writes".into(),
            condition: PolicyCondition::EffectTypeIs(EffectType::WriteExternal),
            effect: PolicyEffect::RequireApproval,
        }];
        assert!(matches!(
            engine.evaluate(&action, &lease, &cap_policies),
            PolicyDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn expired_lease_denied() {
        let engine = PolicyEngine::new();
        let action = make_action("read", vec![EffectType::ReadLocal], false);
        let mut lease = make_lease();
        lease.revoked = true;
        assert!(matches!(
            engine.evaluate(&action, &lease, &[]),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn lease_not_covering_action_denied() {
        let engine = PolicyEngine::new();
        let action = make_action("delete_repo", vec![EffectType::WriteExternal], false);
        let mut lease = make_lease();
        lease.granted_actions = vec!["create_issue".into()];
        assert!(matches!(
            engine.evaluate(&action, &lease, &[]),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn action_matches_pattern() {
        let mut engine = PolicyEngine::new();
        engine.add_global_policy(PolicyRule {
            name: "approve deletes".into(),
            condition: PolicyCondition::ActionMatches {
                pattern: "delete".into(),
            },
            effect: PolicyEffect::RequireApproval,
        });
        let action = make_action("delete_repo", vec![EffectType::WriteExternal], false);
        let lease = make_lease();
        assert!(matches!(
            engine.evaluate(&action, &lease, &[]),
            PolicyDecision::RequireApproval { .. }
        ));

        let action2 = make_action("create_issue", vec![EffectType::WriteExternal], false);
        assert_eq!(
            engine.evaluate(&action2, &lease, &[]),
            PolicyDecision::Allow
        );
    }
}
