//! Declarative predicate language for `Installed`-tier hooks.
//!
//! Extension authors who don't need full programmatic control express their
//! hook as a typed predicate. The host's predicate evaluator (lives in
//! `ironclaw_reborn` follow-up) executes the predicate without invoking any
//! extension code at hook-time, which is both cheaper and structurally safer
//! than running WASM for every capability call.
//!
//! This module defines only the *types*; the evaluator lives elsewhere.

use serde::{Deserialize, Serialize};

/// A complete declarative hook specification, suitable for serialization in
/// an extension manifest's `[[hooks]]` section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookPredicateSpec {
    /// Deny a capability invocation when the predicate matches.
    DenyCapability {
        when: CapabilityPredicate,
        reason: String,
    },
    /// Pause for approval when the predicate matches.
    PauseApproval {
        when: CapabilityPredicate,
        reason: String,
    },
    /// Cap the cumulative value or rate of matching capability calls within a
    /// rolling window.
    RateOrValueCap {
        when: CapabilityPredicate,
        bound: ValueOrRateBound,
        on_exceeded: OnExceededAction,
    },
}

/// A predicate over the capability invocation context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CapabilityPredicate {
    NameEquals {
        name: String,
    },
    NameStartsWith {
        prefix: String,
    },
    All {
        predicates: Vec<CapabilityPredicate>,
    },
    Any {
        predicates: Vec<CapabilityPredicate>,
    },
    /// Always matches. Useful for "deny all of capability X" style rules
    /// paired with a `NameEquals` predicate.
    Always,
}

/// A numeric or rate bound expressed in human-readable form. The evaluator
/// canonicalizes window strings (e.g., "24h", "10m") at evaluation time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ValueOrRateBound {
    /// Maximum N matching invocations in `window`.
    InvocationCount { max: u32, window: String },
    /// Maximum sum of numeric values extracted from `field` across matching
    /// invocations in `window`.
    NumericSum {
        max: String,
        field: String,
        window: String,
    },
}

/// What to do when the bound is exceeded.
///
/// # The `reason` field is for *audit*, not for the model
///
/// Hook authors regularly assume their `reason` text surfaces to the
/// agent loop and to the model. **It does not.** At dispatch time, the
/// model-visible decision carries a closed-vocabulary label
/// (`"hook_predicate_denied"` for `Deny`, `"hook_predicate_pause_requested"`
/// for `PauseApproval`) — the manifest-supplied `reason` is preserved in
/// audit milestones (`HookDecisionEmitted`) but is *never* passed to the
/// model.
///
/// This is intentional. Manifest-supplied strings are author-controlled
/// dynamic input; surfacing them to the model would open a prompt-injection
/// channel (a malicious extension could put instructions in the deny
/// reason). Closed vocabulary at the model boundary closes that channel.
///
/// What `reason` is good for: operator audit, runbook diagnostics,
/// per-decision reporting in the SSE event substrate. Write it for a
/// human reader of the audit log, not for the model.
///
/// See [`crate::kinds::gate::GateDecisionView`] for the closed-vocabulary
/// projection the dispatcher emits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum OnExceededAction {
    Deny { reason: String },
    PauseApproval { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_capability_round_trips_through_json() {
        let spec = HookPredicateSpec::DenyCapability {
            when: CapabilityPredicate::NameStartsWith {
                prefix: "shell.".to_string(),
            },
            reason: "shell denied".to_string(),
        };
        let json = serde_json::to_string(&spec).expect("ser");
        let back: HookPredicateSpec = serde_json::from_str(&json).expect("de");
        assert_eq!(spec, back);
    }

    #[test]
    fn rate_cap_round_trips_through_json() {
        let spec = HookPredicateSpec::RateOrValueCap {
            when: CapabilityPredicate::NameEquals {
                name: "polymarket.place_order".to_string(),
            },
            bound: ValueOrRateBound::InvocationCount {
                max: 10,
                window: "24h".to_string(),
            },
            on_exceeded: OnExceededAction::Deny {
                reason: "daily cap".to_string(),
            },
        };
        let json = serde_json::to_string(&spec).expect("ser");
        let back: HookPredicateSpec = serde_json::from_str(&json).expect("de");
        assert_eq!(spec, back);
    }

    #[test]
    fn nested_predicate_round_trips() {
        let spec = HookPredicateSpec::DenyCapability {
            when: CapabilityPredicate::All {
                predicates: vec![
                    CapabilityPredicate::NameStartsWith {
                        prefix: "wallet.".to_string(),
                    },
                    CapabilityPredicate::Any {
                        predicates: vec![
                            CapabilityPredicate::NameEquals {
                                name: "wallet.sign".to_string(),
                            },
                            CapabilityPredicate::NameEquals {
                                name: "wallet.approve".to_string(),
                            },
                        ],
                    },
                ],
            },
            reason: "wallet ops disabled".to_string(),
        };
        let json = serde_json::to_string(&spec).expect("ser");
        let back: HookPredicateSpec = serde_json::from_str(&json).expect("de");
        assert_eq!(spec, back);
    }
}
