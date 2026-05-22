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

/// Maximum length, in bytes, of a free-form `audit_reason` after sanitization.
/// Reasons longer than this are truncated and a `…` ellipsis is appended.
/// Bounds memory/network amplification across the milestone/SSE/audit path
/// against a hostile manifest emitting a multi-megabyte `reason` field.
pub const MAX_AUDIT_REASON_BYTES: usize = 512;

/// Sanitize a manifest- or hook-supplied free-form audit reason before it
/// crosses the milestone/SSE/audit boundary. Strips ASCII/Unicode control
/// characters (other than space) and caps the length at
/// [`MAX_AUDIT_REASON_BYTES`]. Control-character stripping prevents log
/// injection (CR/LF, ANSI escapes) downstream of the dispatcher; length cap
/// prevents memory/network DoS.
pub fn sanitize_audit_reason(reason: Option<String>) -> Option<String> {
    let raw = reason?;
    let mut out = String::with_capacity(raw.len().min(MAX_AUDIT_REASON_BYTES));
    for ch in raw.chars() {
        if ch == ' ' || !ch.is_control() {
            // Stop appending if we'd exceed the cap; leave room for the
            // ellipsis below.
            if out.len() + ch.len_utf8() > MAX_AUDIT_REASON_BYTES {
                out.push('…');
                break;
            }
            out.push(ch);
        }
    }
    if out.is_empty() { None } else { Some(out) }
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
    fn sanitize_audit_reason_truncates_oversized_input() {
        let huge = "x".repeat(MAX_AUDIT_REASON_BYTES * 4);
        let out = sanitize_audit_reason(Some(huge)).expect("non-empty");
        // Output must fit within the cap (plus the trailing ellipsis, which
        // is itself bounded by `MAX_AUDIT_REASON_BYTES`).
        assert!(
            out.len() <= MAX_AUDIT_REASON_BYTES + '…'.len_utf8(),
            "len={}",
            out.len()
        );
        assert!(out.ends_with('…'));
    }

    #[test]
    fn sanitize_audit_reason_strips_control_chars() {
        let raw = "ok\r\n\x1b[31mred\x1b[0m\ttab".to_string();
        let out = sanitize_audit_reason(Some(raw)).expect("non-empty");
        // No CR/LF/ESC/TAB should survive; the ANSI body letters do.
        assert!(!out.contains('\r'));
        assert!(!out.contains('\n'));
        assert!(!out.contains('\x1b'));
        assert!(!out.contains('\t'));
        assert!(out.contains("red"));
    }

    #[test]
    fn sanitize_audit_reason_preserves_normal_text() {
        let raw = "polymarket daily cap exceeded".to_string();
        assert_eq!(
            sanitize_audit_reason(Some(raw.clone())),
            Some(raw),
            "normal text passes through unchanged"
        );
    }

    #[test]
    fn sanitize_audit_reason_returns_none_for_empty_or_only_control() {
        assert_eq!(sanitize_audit_reason(None), None);
        assert_eq!(sanitize_audit_reason(Some(String::new())), None);
        assert_eq!(sanitize_audit_reason(Some("\r\n\t".to_string())), None);
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
