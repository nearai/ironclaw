use ironclaw_host_api::{
    decision::DenyReason,
    dispatch::INPUT_ENCODE_HUMAN_SUMMARY,
    resolution::{Outcome, Resolution},
    result_meta::{
        CapabilityRecoveryHint, FailureKind, ModelFailureDiagnostic, ModelInputIssue,
        SameCallRetryConstraint,
    },
};
use ironclaw_loop_contracts::{
    CapabilityFailure, CapabilityFailureDetail, CapabilityInputIssue, ModelVisibleToolObservation,
    ToolObservationDetail,
};

use super::{model_visible_capability_failure_observation, sanitized_strategy_summary_or_fallback};
use crate::strategies::{CapabilityErrorSummary, SanitizedStrategySummary};

const MAX_SAFE_SUMMARY_BYTES: usize = 512;
const STRATEGY_INPUT_COULD_NOT_BE_ENCODED_SUMMARY: &str = "input could not be encoded";

/// Strategy-visible summary for a capability-stage port `Err` whose kind is
/// NOT a terminal host fault (`capability_port_error_is_terminal` == false).
/// The kind projection is owned by `AgentLoopHostErrorKind::failure_kind`;
/// the summary text fail-softs through `capability_failed_summary` (a summary
/// that trips the strict validator degrades to a canned fallback instead of
/// borking the run).
pub(super) fn capability_port_error_summary(
    error: &ironclaw_loop_contracts::AgentLoopHostError,
) -> CapabilityErrorSummary {
    let kind = error.kind.failure_kind();
    CapabilityErrorSummary {
        kind,
        safe_summary: capability_failed_summary(kind, error.safe_summary.clone()),
    }
}

/// Model-visible observation for a recoverable capability-stage port `Err`.
/// Carries the port error's secret-scrubbed `detail` (when present) so the
/// model can retry or explain instead of guessing from the kind alone.
fn capability_failure_from_port_error(
    error: &ironclaw_loop_contracts::AgentLoopHostError,
) -> CapabilityFailure {
    let detail = error.detail.clone().unwrap_or_else(|| {
        ironclaw_loop_contracts::sanitize_model_visible_text(error.safe_summary.clone())
    });
    CapabilityFailure {
        error_kind: error.kind.failure_kind(),
        safe_summary: error.safe_summary.clone(),
        detail: CapabilityFailureDetail::Diagnostic { text: detail },
    }
}

pub(super) fn recoverable_port_error_resolution(
    error: ironclaw_loop_contracts::AgentLoopHostError,
) -> Resolution {
    let failure = capability_failure_from_port_error(&error);
    ironclaw_loop_contracts::resolution::failed(
        failure.error_kind,
        failure.safe_summary,
        failure.detail,
    )
}

/// Model-visible observation for a recoverable capability-stage port `Err`.
/// Carries the port error's secret-scrubbed `detail` (when present) so the
/// model can retry or explain instead of guessing from the kind alone.
pub(super) fn capability_port_error_observation(
    error: &ironclaw_loop_contracts::AgentLoopHostError,
) -> ModelVisibleToolObservation {
    model_visible_capability_failure_observation(&capability_failure_from_port_error(error))
}

pub(super) fn capability_failed_summary(
    error_kind: FailureKind,
    safe_summary: String,
) -> SanitizedStrategySummary {
    prefixed_capability_summary(
        format!("capability failed with {}: ", error_kind.as_str()),
        safe_summary,
    )
}

pub(super) fn capability_denied_summary(
    reason_kind: &str,
    safe_summary: String,
) -> SanitizedStrategySummary {
    prefixed_capability_summary(
        format!("capability denied with {reason_kind}: "),
        safe_summary,
    )
}

fn prefixed_capability_summary(prefix: String, safe_summary: String) -> SanitizedStrategySummary {
    let safe_summary = strategy_safe_capability_summary_detail(safe_summary);
    // Fail soft: a capability summary that fails strict validation degrades to
    // a fixed fallback instead of aborting the run. The real cause still rides
    // the model-visible Diagnostic detail via the paired observation, so no
    // information is lost by degrading the card summary here.
    let (detail, _) = sanitized_strategy_summary_or_fallback(
        safe_summary,
        "the tool failure details were redacted",
    );
    let detail = truncate_summary_detail(
        detail.as_str(),
        MAX_SAFE_SUMMARY_BYTES.saturating_sub(prefix.len()),
    );
    // The combined summary must also fail soft: the prefix itself can trip the
    // validator (`Failed(Authorization)` yields "capability failed with
    // authorization: ", and "authorization:" is a banned marker). A recoverable
    // tool failure must never turn terminal because of its own card label.
    let (summary, _) = sanitized_strategy_summary_or_fallback(
        format!("{prefix}{detail}"),
        "the tool failure details were redacted",
    );
    summary
}

/// Extract the secret-scrubbed model-visible diagnostic text from a tool
/// observation, if any, so a terminal capability failure can carry the real
/// cause on `SanitizedFailure.detail` for the failure explainer.
pub(super) fn model_observation_diagnostic_detail(
    observation: Option<&ModelVisibleToolObservation>,
) -> Option<String> {
    match observation.map(|observation| &observation.detail) {
        Some(ToolObservationDetail::GenericFailure {
            detail: Some(detail),
            ..
        }) => Some(detail.clone()),
        _ => None,
    }
}

fn strategy_safe_capability_summary_detail(safe_summary: String) -> String {
    if safe_summary == INPUT_ENCODE_HUMAN_SUMMARY {
        STRATEGY_INPUT_COULD_NOT_BE_ENCODED_SUMMARY.to_string()
    } else {
        safe_summary
    }
}

fn truncate_summary_detail(detail: &str, max_bytes: usize) -> &str {
    if detail.len() <= max_bytes {
        return detail;
    }
    let mut end = max_bytes;
    while end > 0 && !detail.is_char_boundary(end) {
        end -= 1;
    }
    &detail[..end]
}

pub(super) fn capability_failure_from_recoverable(
    error_kind: &FailureKind,
    diagnostic: &ModelFailureDiagnostic,
    outcome: &Outcome,
) -> CapabilityFailure {
    CapabilityFailure {
        // The verdict already carries the unified kind; no tag round-trip.
        error_kind: *error_kind,
        safe_summary: outcome.summary.as_str().to_string(),
        detail: capability_failure_detail_from(diagnostic),
    }
}

fn capability_failure_detail_from(diagnostic: &ModelFailureDiagnostic) -> CapabilityFailureDetail {
    match diagnostic {
        ModelFailureDiagnostic::InvalidInput { issues } => CapabilityFailureDetail::InvalidInput {
            issues: issues
                .as_slice()
                .iter()
                .map(capability_input_issue_from)
                .collect(),
        },
        ModelFailureDiagnostic::Diagnostic { text } => CapabilityFailureDetail::Diagnostic {
            text: text.as_str().to_string(),
        },
        ModelFailureDiagnostic::HostRemediation { text } => {
            CapabilityFailureDetail::HostRemediation { text: text.clone() }
        }
    }
}

fn capability_input_issue_from(issue: &ModelInputIssue) -> CapabilityInputIssue {
    CapabilityInputIssue {
        path: issue.path.as_str().to_string(),
        code: issue.code,
        expected: issue
            .expected
            .as_ref()
            .map(|value| value.as_str().to_string()),
        received: issue
            .received
            .as_ref()
            .map(|value| value.as_str().to_string()),
        schema_path: issue
            .schema_path
            .as_ref()
            .map(|value| value.as_str().to_string()),
    }
}

/// What the model should do about a denial, and whether re-issuing the same
/// call could ever work.
///
/// Denials reached the model with `model_observation: None` — no recovery, no
/// retry constraint, no repairs — so a denial meaning *authenticate and this
/// succeeds* was indistinguishable from a permanent block (#6284 item 4).
/// #6781 made the *reason* specific; this makes the reason **actionable**.
///
/// Exhaustive and wildcard-free: a new [`DenyReason`] cannot compile until its
/// next move is chosen.
pub(super) fn deny_recovery(
    reason: DenyReason,
) -> (SameCallRetryConstraint, CapabilityRecoveryHint) {
    match reason {
        // A credential unlocks it — the same call succeeds afterwards.
        DenyReason::UnknownSecret => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::AuthenticateThenRetry,
        ),
        // A human unlocks it. Worth asking; the same call may then succeed.
        DenyReason::ApprovalDenied => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::RequestApproval,
        ),
        // A grant is missing. Not the model's to fix by rewording.
        DenyReason::MissingGrant => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::RequestApproval,
        ),
        // Not this caller's capability; re-calling cannot succeed.
        DenyReason::UnknownCapability => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::UseDifferentCapability,
        ),
        // The model named a path it can correct.
        DenyReason::InvalidPath | DenyReason::PathOutsideMount => (
            SameCallRetryConstraint::RequiresChangedInput,
            CapabilityRecoveryHint::CorrectArgumentsBeforeRetry,
        ),
        // Capacity, not permission: waiting is the move.
        DenyReason::BudgetDenied | DenyReason::ResourceLimitExceeded => (
            SameCallRetryConstraint::AllowedAfterDelay,
            CapabilityRecoveryHint::WaitThenRetry,
        ),
        // Permanently refused.
        DenyReason::NetworkDenied | DenyReason::PolicyDenied => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::ReviseApproach,
        ),
        // A host fault, not a refusal. Nothing the model can unlock, but the
        // same call is not forbidden on principle.
        DenyReason::InternalInvariantViolation => (
            SameCallRetryConstraint::NotUseful,
            CapabilityRecoveryHint::RespectFailureConstraint,
        ),
    }
}

/// Fixed, host-authored one-line summary for a denial observation.
///
/// Deliberately not the capability's own text: the summary channel is strictly
/// validated and a denial's `safe_summary` may degrade to a placeholder. The
/// untrusted cause rides `detail` instead, where the lenient validator applies.
pub(super) fn capability_denied_observation_summary(reason_kind: &str) -> String {
    format!("The capability was denied ({reason_kind}).")
}

/// The denial's own text for the model-visible detail channel. Empty summaries
/// become an actionable host-authored sentence rather than a category-only
/// observation.
pub(super) fn denial_detail_text(reason_kind: &str, safe_summary: &str) -> String {
    let trimmed = safe_summary.trim();
    if trimmed.is_empty() {
        format!(
            "The host denied this capability with reason {reason_kind}; follow the recovery guidance before choosing the next action."
        )
    } else {
        trimmed.to_string()
    }
}

pub(super) fn deny_reason_tag(reason: DenyReason) -> &'static str {
    match reason {
        DenyReason::MissingGrant => "missing_grant",
        DenyReason::InvalidPath => "invalid_path",
        DenyReason::PathOutsideMount => "path_outside_mount",
        DenyReason::UnknownCapability => "unknown_capability",
        DenyReason::UnknownSecret => "unknown_secret",
        DenyReason::NetworkDenied => "network_denied",
        DenyReason::BudgetDenied => "budget_denied",
        DenyReason::ApprovalDenied => "approval_denied",
        DenyReason::PolicyDenied => "policy_denied",
        DenyReason::ResourceLimitExceeded => "resource_limit_exceeded",
        DenyReason::InternalInvariantViolation => "internal_invariant_violation",
    }
}

#[cfg(test)]
mod tests;
