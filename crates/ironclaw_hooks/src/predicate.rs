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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum OnExceededAction {
    /// Deny with a free-form `reason` (audit-only). Model-visible label
    /// collapses to the static `hook_predicate_denied` — see the type-
    /// level doc above.
    Deny { reason: String },
    /// Deny with both a closed-vocabulary [`DenyReasonCode`] **and** a
    /// free-form audit `reason`. The model-visible label becomes the
    /// code's static string (e.g. `rate_limit`, `value_cap`) so the
    /// agent can distinguish *why* a hook denied without opening a
    /// prompt-injection channel through the free-form reason.
    DenyWithCode {
        code: DenyReasonCode,
        reason: String,
    },
    /// Pause for human approval with a free-form audit `reason`.
    PauseApproval { reason: String },
    /// Pause for human approval, with a closed-vocabulary
    /// [`PauseReasonCode`] surfaced to the model alongside the audit-only
    /// `reason`.
    PauseApprovalWithCode {
        code: PauseReasonCode,
        reason: String,
    },
}

/// Closed-vocabulary reason a hook denied a capability invocation.
///
/// Hook authors who want to communicate *why* a deny happened use
/// [`OnExceededAction::DenyWithCode`]; the dispatcher surfaces the
/// code's static string (via [`Self::as_label`]) as the model-visible
/// label, replacing the generic `hook_predicate_denied`. The free-form
/// `reason` payload still stays audit-only.
///
/// The vocabulary is intentionally coarse-grained. Labels are *machine-
/// readable identifiers*, not localized strings — the UI layer maps them
/// to user-facing text. Adding new variants is a back-compat
/// consideration: downstream agents may pattern-match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReasonCode {
    /// Default; matches today's `hook_predicate_denied` label. Use this
    /// when no more specific variant fits.
    Generic,
    /// Per-time-window rate limit tripped (e.g., `InvocationCount` cap).
    RateLimit,
    /// Per-time-window numeric-sum cap tripped (e.g., $-denominated
    /// total).
    ValueCap,
    /// Capability is on a deny-list configured by the extension.
    Blocklist,
    /// Capability needs human approval before proceeding. Pair with
    /// `PauseApprovalWithCode { code: PauseReasonCode::RequiresApproval }`
    /// when the hook *requests* approval; use this variant when the hook
    /// *denies* an invocation that would require approval the user
    /// hasn't granted.
    RequiresApproval,
    /// Capability is outside the configured policy envelope (catch-all
    /// for policy denials that don't fit the more specific variants).
    OutOfPolicy,
}

impl DenyReasonCode {
    /// Stable, model-visible string for this code. Stays in the closed
    /// vocabulary the dispatcher's sink accepts (`&'static str`).
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Generic => "hook_predicate_denied",
            Self::RateLimit => "hook_rate_limit",
            Self::ValueCap => "hook_value_cap",
            Self::Blocklist => "hook_blocklist",
            Self::RequiresApproval => "hook_requires_approval",
            Self::OutOfPolicy => "hook_out_of_policy",
        }
    }
}

/// Closed-vocabulary reason a hook is requesting a pause for human
/// approval. Mirrors [`DenyReasonCode`]; see that type for the
/// rationale on the closed-vocab design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReasonCode {
    /// Default; matches today's `hook_predicate_pause_requested` label.
    Generic,
    /// The user must explicitly approve this capability before it runs.
    RequiresApproval,
    /// The action exceeds a policy threshold (rate or value) and needs
    /// human review.
    OverThreshold,
    /// The action is sensitive enough that the extension wants explicit
    /// user confirmation regardless of any threshold.
    SensitiveAction,
}

impl PauseReasonCode {
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Generic => "hook_predicate_pause_requested",
            Self::RequiresApproval => "hook_pause_requires_approval",
            Self::OverThreshold => "hook_pause_over_threshold",
            Self::SensitiveAction => "hook_pause_sensitive_action",
        }
    }
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

    /// Each `DenyReasonCode` variant must produce a stable, non-empty
    /// model-visible label. Pins the vocabulary so a future enum-variant
    /// rename or label rewrite is loud.
    #[test]
    fn deny_reason_code_labels_are_stable() {
        let pairs: Vec<(DenyReasonCode, &'static str)> = vec![
            (DenyReasonCode::Generic, "hook_predicate_denied"),
            (DenyReasonCode::RateLimit, "hook_rate_limit"),
            (DenyReasonCode::ValueCap, "hook_value_cap"),
            (DenyReasonCode::Blocklist, "hook_blocklist"),
            (DenyReasonCode::RequiresApproval, "hook_requires_approval"),
            (DenyReasonCode::OutOfPolicy, "hook_out_of_policy"),
        ];
        for (code, expected) in pairs {
            assert_eq!(
                code.as_label(),
                expected,
                "label for {code:?} changed — this is a cross-crate \
                 wire-format break for any consumer pattern-matching on \
                 the model-visible deny label"
            );
        }
    }

    #[test]
    fn pause_reason_code_labels_are_stable() {
        let pairs: Vec<(PauseReasonCode, &'static str)> = vec![
            (PauseReasonCode::Generic, "hook_predicate_pause_requested"),
            (
                PauseReasonCode::RequiresApproval,
                "hook_pause_requires_approval",
            ),
            (PauseReasonCode::OverThreshold, "hook_pause_over_threshold"),
            (
                PauseReasonCode::SensitiveAction,
                "hook_pause_sensitive_action",
            ),
        ];
        for (code, expected) in pairs {
            assert_eq!(code.as_label(), expected);
        }
    }

    /// `OnExceededAction::DenyWithCode` round-trips through serde. The
    /// closed-vocabulary code is serialized as a snake_case string;
    /// downstream manifest authors can use the new variant alongside the
    /// legacy `Deny { reason }` shape.
    #[test]
    fn deny_with_code_round_trips_through_json() {
        let action = OnExceededAction::DenyWithCode {
            code: DenyReasonCode::RateLimit,
            reason: "polymarket daily cap exceeded".to_string(),
        };
        let json = serde_json::to_string(&action).expect("ser");
        let back: OnExceededAction = serde_json::from_str(&json).expect("de");
        assert_eq!(action, back);
        // Defense against accidental enum-tag drift: the wire shape
        // must serialize the code variant as snake_case.
        assert!(json.contains("\"rate_limit\""), "wire form: {json}");
    }

    #[test]
    fn pause_approval_with_code_round_trips_through_json() {
        let action = OnExceededAction::PauseApprovalWithCode {
            code: PauseReasonCode::OverThreshold,
            reason: "$1000/24h threshold tripped".to_string(),
        };
        let json = serde_json::to_string(&action).expect("ser");
        let back: OnExceededAction = serde_json::from_str(&json).expect("de");
        assert_eq!(action, back);
        assert!(json.contains("\"over_threshold\""), "wire form: {json}");
    }

    /// Threat-model regression: a hook author cannot smuggle arbitrary
    /// text into the model-visible label via `DenyWithCode`. The `code`
    /// field is typed as the enum — no `String` slot for the model-side
    /// label exists on the closed-vocab path.
    #[test]
    fn deny_with_code_only_exposes_enum_variants_to_model() {
        // This is a compile-time property; the test exists to document
        // the intent and to fail if a future refactor turns `code` into
        // a `String` field.
        let _check: fn(OnExceededAction) -> Option<DenyReasonCode> = |action| match action {
            OnExceededAction::DenyWithCode { code, .. } => Some(code),
            _ => None,
        };
    }
}
