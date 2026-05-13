//! Conversions from hook-crate types into the closed-vocabulary milestone
//! summaries defined in `ironclaw_turns`.
//!
//! The `ironclaw_turns` crate cannot depend on `ironclaw_hooks` (that boundary
//! is enforced by `ironclaw_architecture`). To let the dispatcher emit
//! milestones into a `LoopHostMilestoneSink` without leaking hook-internal
//! types across the seam, this module produces the string-shaped
//! representations the milestone sink expects.

use ironclaw_turns::run_profile::HookDecisionSummary;

use crate::failure_policy::{FailureCategory, FailureDisposition};
use crate::identity::HookId;
use crate::kinds::gate::{BeforeCapabilityHookDecision, GateDecisionInner};
use crate::registry::HookPointSpec;
use crate::trust::HookTrustClass;

/// Render a [`HookId`] into the wire form used by milestones.
pub fn hook_id_string(hook_id: HookId) -> String {
    hook_id.to_hex()
}

/// Stable string label for a [`HookTrustClass`].
pub fn trust_class_label(class: HookTrustClass) -> &'static str {
    match class {
        HookTrustClass::Builtin => "builtin",
        HookTrustClass::Trusted => "trusted",
        HookTrustClass::Installed => "installed",
        HookTrustClass::SelfAuthored => "self_authored",
    }
}

/// Stable string label for a [`HookPointSpec`].
pub fn point_label(point: HookPointSpec) -> &'static str {
    match point {
        HookPointSpec::BeforeCapability => "before_capability",
        HookPointSpec::BeforePrompt => "before_prompt",
        HookPointSpec::AfterModel => "after_model",
        HookPointSpec::AfterCapability => "after_capability",
        HookPointSpec::AfterCheckpoint => "after_checkpoint",
    }
}

/// Stable string label for a [`FailureCategory`].
pub fn failure_category_label(category: FailureCategory) -> &'static str {
    match category {
        FailureCategory::Timeout => "timeout",
        FailureCategory::Panic => "panic",
        FailureCategory::Malformed => "malformed",
        FailureCategory::AttenuationViolation => "attenuation_violation",
    }
}

/// Stable string label for a [`FailureDisposition`].
pub fn failure_disposition_label(disposition: FailureDisposition) -> &'static str {
    match disposition {
        FailureDisposition::FailClosed => "fail_closed",
        FailureDisposition::FailIsolated => "fail_isolated",
    }
}

/// Convert a [`BeforeCapabilityHookDecision`] into the closed-vocabulary
/// summary published over the milestone sink. The sanitized reason is
/// stringified at the seam because the strongly-typed `SanitizedReason` lives
/// in this crate and cannot cross into `ironclaw_turns`.
pub fn gate_decision_summary(decision: &BeforeCapabilityHookDecision) -> HookDecisionSummary {
    match &decision.inner {
        GateDecisionInner::Allow => HookDecisionSummary::Allow,
        GateDecisionInner::Deny { reason } => HookDecisionSummary::Deny {
            reason: reason.as_str().to_string(),
        },
        GateDecisionInner::PauseApproval { reason } => HookDecisionSummary::PauseApproval {
            reason: reason.as_str().to_string(),
        },
        GateDecisionInner::PauseAuth { reason } => HookDecisionSummary::PauseAuth {
            reason: reason.as_str().to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SanitizedReason;
    use crate::identity::HookVersion;

    #[test]
    fn labels_are_stable() {
        assert_eq!(trust_class_label(HookTrustClass::Builtin), "builtin");
        assert_eq!(trust_class_label(HookTrustClass::Installed), "installed");
        assert_eq!(
            point_label(HookPointSpec::BeforeCapability),
            "before_capability"
        );
        assert_eq!(failure_category_label(FailureCategory::Timeout), "timeout");
        assert_eq!(
            failure_disposition_label(FailureDisposition::FailClosed),
            "fail_closed"
        );
    }

    #[test]
    fn hook_id_round_trip() {
        let id = HookId::for_builtin("test::path", HookVersion::ONE);
        let hex = hook_id_string(id);
        assert_eq!(hex.len(), 64, "blake3 hex is 64 chars");
        assert_eq!(hex, id.to_hex());
    }

    /// Cross-crate contract: the conversion path used at the milestone
    /// boundary (`telemetry::hook_id_string`) must produce byte-for-byte the
    /// same output as `HookId::to_hex()`. Downstream `ironclaw_turns`
    /// consumers (SSE, audit, replay) key on this exact string. If
    /// `hook_id_string` ever diverges from `to_hex` (e.g. someone tries to
    /// add a prefix at the seam), this test catches it.
    #[test]
    fn hook_id_string_serialization_matches_to_hex() {
        let ids = [
            HookId::for_builtin("crate::a::b", HookVersion::ONE),
            HookId::for_builtin("crate::a::b", HookVersion(2)),
            HookId::derive(
                &crate::identity::ExtensionId("ext".to_string()),
                "1.0",
                &crate::identity::HookLocalId("h".to_string()),
                HookVersion::ONE,
            ),
        ];
        for id in ids {
            assert_eq!(
                hook_id_string(id),
                id.to_hex(),
                "telemetry::hook_id_string must match HookId::to_hex byte-for-byte"
            );
        }
    }

    #[test]
    fn allow_decision_summary() {
        let allow = BeforeCapabilityHookDecision::allow();
        assert_eq!(gate_decision_summary(&allow), HookDecisionSummary::Allow);
    }

    #[test]
    fn deny_decision_summary_carries_reason() {
        let deny = BeforeCapabilityHookDecision::deny(SanitizedReason::from_static("nope"));
        assert_eq!(
            gate_decision_summary(&deny),
            HookDecisionSummary::Deny {
                reason: "nope".to_string()
            }
        );
    }
}
