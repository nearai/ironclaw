//! Sanitized agent-loop host error type, its kinds/reason-kinds, and the shared
//! `unsupported host method` constructor used by fail-closed port defaults.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use ironclaw_host_api::turn::LoopGateRef;

use super::{model::LoopModelUsage, refs::LoopSafeSummary};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopHostErrorKind {
    Unauthorized,
    /// Host-owned credential acquisition failed for the requested provider/model.
    /// The error summary must stay sanitized and must not expose secret material,
    /// token refresh details, or backend-specific credential-store errors.
    CredentialUnavailable,
    ScopeMismatch,
    StaleSurface,
    InvalidInvocation,
    /// The request payload itself is well-formed but its content is invalid in
    /// the current host state (e.g. schema id/version mismatch on checkpoint load).
    Invalid,
    /// The model/provider output was structurally invalid for the active loop contract.
    InvalidOutput,
    /// The provider refused to produce the completion because its content
    /// filter rejected the request or response.
    ContentFiltered,
    PolicyDenied,
    /// Generic non-model resource/capacity exhaustion. Model-call budget and
    /// token-window outcomes use the three precise variants below.
    BudgetExceeded,
    /// The configured host-side spend budget cannot admit another model call.
    SpendBudgetExceeded,
    /// The model input exceeded the provider's context window.
    ContextOverflow,
    /// The provider stopped because generated output hit its token ceiling.
    OutputTruncated,
    /// The model call would push utilization past the configured pause
    /// threshold. Callers surface an approval gate (foreground or
    /// background) and retry after the user resolves it.
    BudgetApprovalRequired,
    /// Durable budget accounting (reservation read/write/reconcile)
    /// failed. Distinct from `BudgetExceeded`/`BudgetApprovalRequired`
    /// because the failure is in the governor itself, not in the budget
    /// outcome — callers must fail closed.
    BudgetAccountingFailed,
    /// The model provider throttled the request. The optional typed retry
    /// delay on [`AgentLoopHostError`] controls same-route backoff.
    RateLimited,
    Unavailable,
    Cancelled,
    CheckpointRejected,
    TranscriptWriteFailed,
    Internal,
}

impl AgentLoopHostErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthorized => "unauthorized",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::ScopeMismatch => "scope_mismatch",
            Self::StaleSurface => "stale_surface",
            Self::InvalidInvocation => "invalid_invocation",
            Self::Invalid => "invalid",
            Self::InvalidOutput => "invalid_output",
            Self::ContentFiltered => "content_filtered",
            Self::PolicyDenied => "policy_denied",
            Self::BudgetExceeded => "budget_exceeded",
            Self::SpendBudgetExceeded => "spend_budget_exceeded",
            Self::ContextOverflow => "context_overflow",
            Self::OutputTruncated => "output_truncated",
            Self::BudgetApprovalRequired => "budget_approval_required",
            Self::BudgetAccountingFailed => "budget_accounting_failed",
            Self::RateLimited => "rate_limited",
            Self::Unavailable => "unavailable",
            Self::Cancelled => "cancelled",
            Self::CheckpointRejected => "checkpoint_rejected",
            Self::TranscriptWriteFailed => "transcript_write_failed",
            Self::Internal => "internal",
        }
    }

    /// Project this loop-host error kind onto the unified closed
    /// [`ironclaw_host_api::result_meta::FailureKind`] vocabulary. Exhaustive on purpose: a
    /// new `AgentLoopHostErrorKind` variant must pick its honest failure kind
    /// here instead of falling into a wildcard bucket. Decision sites ask
    /// [`ironclaw_host_api::result_meta::FailureKind::fate`] for the disposition.
    pub fn failure_kind(self) -> ironclaw_host_api::result_meta::FailureKind {
        use ironclaw_host_api::result_meta::FailureKind;
        match self {
            Self::Unauthorized => FailureKind::Authorization,
            Self::CredentialUnavailable => FailureKind::AuthRequired,
            Self::ScopeMismatch => FailureKind::Authorization,
            Self::StaleSurface => FailureKind::StaleSurface,
            Self::InvalidInvocation | Self::Invalid => FailureKind::InputEncode,
            Self::InvalidOutput => FailureKind::OutputDecode,
            Self::ContentFiltered => FailureKind::OperationFailed,
            Self::PolicyDenied => FailureKind::PolicyDenied,
            Self::BudgetExceeded | Self::SpendBudgetExceeded | Self::ContextOverflow => {
                FailureKind::Resource
            }
            Self::OutputTruncated => FailureKind::OutputTooLarge,
            // "Callers surface an approval gate and retry after the user
            // resolves it" (variant doc) — a PARK semantic, so the projection
            // must carry the Park-fated kind, not a model-visible tool error.
            Self::BudgetApprovalRequired => FailureKind::AuthRequired,
            // The budget governor itself failed — "callers must fail closed"
            // (variant doc). Not a budget outcome and not classifiable as one:
            // the non-retryable unclassified sink keeps it from being retried
            // or mistaken for a quota the model can route around.
            Self::BudgetAccountingFailed => FailureKind::Unclassified,
            Self::RateLimited => FailureKind::Transient,
            Self::Unavailable => FailureKind::Unavailable,
            Self::Cancelled => FailureKind::Cancelled,
            // A rejected checkpoint (schema id/version mismatch) is
            // deterministic — it cannot succeed on re-attempt, so it must not
            // ride the retryable `Internal` bucket and burn retry budget.
            Self::CheckpointRejected => FailureKind::OperationFailed,
            Self::TranscriptWriteFailed | Self::Internal => FailureKind::Internal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopHostErrorReasonKind {
    ModelCreditsExhausted,
}

impl AgentLoopHostErrorReasonKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelCreditsExhausted => "model_credits_exhausted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
#[error("agent loop host {kind:?}: {safe_summary}")]
pub struct AgentLoopHostError {
    pub kind: AgentLoopHostErrorKind,
    pub safe_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_kind: Option<AgentLoopHostErrorReasonKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_ref: Option<LoopGateRef>,
    /// Provider-supplied retry delay in milliseconds. This stays typed across
    /// the host boundary so retry policy never parses diagnostic prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    /// Deterministic evidence that the ordered model-provider chain has another
    /// route available for recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fallback_index: Option<u32>,
    /// Provider-reported usage for a call that consumed tokens before failing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<LoopModelUsage>,
    /// Model-visible, secret-scrubbed raw cause. Unlike `safe_summary`, this
    /// carries the original error text (paths, codes, schema refs) so the model
    /// can retry or explain. Secret VALUES are redacted by the producer via
    /// [`sanitize_model_visible_text`](super::sanitize_model_visible_text); the
    /// word/delimiter ban is NOT applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AgentLoopHostError {
    pub fn new(kind: AgentLoopHostErrorKind, safe_summary: impl Into<String>) -> Self {
        Self {
            kind,
            safe_summary: safe_summary.into(),
            reason_kind: None,
            gate_ref: None,
            retry_after_ms: None,
            next_fallback_index: None,
            usage: None,
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_reason_kind(mut self, reason_kind: AgentLoopHostErrorReasonKind) -> Self {
        self.reason_kind = Some(reason_kind);
        self
    }

    pub fn with_gate_ref(mut self, gate_ref: LoopGateRef) -> Self {
        self.gate_ref = Some(gate_ref);
        self
    }
    pub fn with_retry_after_ms(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub fn with_next_fallback_index(mut self, fallback_index: u32) -> Self {
        self.next_fallback_index = Some(fallback_index);
        self
    }

    pub fn with_usage(mut self, usage: LoopModelUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Apply the fail-closed surface policy for transcript-write failures.
    ///
    /// Backend diagnostics at this boundary may contain raw assistant content
    /// or credentials. Keep the typed kind and other structured fields, but
    /// replace the summary with the fixed host-authored cause and drop detail.
    /// Other host-error kinds are returned unchanged.
    pub fn sanitize_transcript_write_failure(mut self) -> Self {
        if self.kind == AgentLoopHostErrorKind::TranscriptWriteFailed {
            self.safe_summary = LoopSafeSummary::assistant_transcript_write_failed()
                .as_str()
                .to_string();
            self.detail = None;
        }
        self
    }
}

pub(crate) fn unsupported_host_method(method: &'static str) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        format!("agent loop host method {method} is unavailable"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_loop_host_error_carries_optional_detail() {
        let path = "missing input_schema_ref at /system/extensions/google-calendar/list_calendars.input.v1.json";
        let error = AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "host runtime rejected capability request",
        )
        .with_detail(path);
        assert_eq!(error.detail.as_deref(), Some(path));

        let plain = AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, "boom");
        assert_eq!(plain.detail, None);
    }

    #[test]
    fn transcript_write_surface_sanitization_is_scoped_to_transcript_failures() {
        let transcript = AgentLoopHostError::new(
            AgentLoopHostErrorKind::TranscriptWriteFailed,
            "backend rejected raw assistant content",
        )
        .with_detail("storage credential sk-secret")
        .sanitize_transcript_write_failure();
        assert_eq!(
            transcript.safe_summary,
            LoopSafeSummary::assistant_transcript_write_failed().as_str()
        );
        assert_eq!(transcript.detail, None);

        let scope = AgentLoopHostError::new(
            AgentLoopHostErrorKind::ScopeMismatch,
            "thread scope did not match",
        )
        .with_detail("expected tenant scope")
        .sanitize_transcript_write_failure();
        assert_eq!(scope.safe_summary, "thread scope did not match");
        assert_eq!(scope.detail.as_deref(), Some("expected tenant scope"));
    }

    /// Regression (#6684 review): the budget/checkpoint port kinds must not
    /// collapse into one model-visible `Resource`/retryable `Internal` bucket —
    /// each projects the fate its variant doc prescribes.
    #[test]
    fn budget_and_checkpoint_port_errors_project_honest_fates() {
        use ironclaw_host_api::result_meta::{FailureFate, FailureKind};
        // Park semantic: "callers surface an approval gate ... and retry
        // after the user resolves it" — not a tool error to route around.
        assert_eq!(
            AgentLoopHostErrorKind::BudgetApprovalRequired.failure_kind(),
            FailureKind::AuthRequired
        );
        assert_eq!(
            AgentLoopHostErrorKind::BudgetApprovalRequired
                .failure_kind()
                .fate(),
            FailureFate::Park
        );
        // Fail closed: a governor fault is neither a quota outcome nor
        // retryable.
        assert_eq!(
            AgentLoopHostErrorKind::BudgetAccountingFailed.failure_kind(),
            FailureKind::Unclassified
        );
        assert!(
            !AgentLoopHostErrorKind::BudgetAccountingFailed
                .failure_kind()
                .is_retryable()
        );
        // A schema id/version rejection is deterministic — never retryable.
        assert_eq!(
            AgentLoopHostErrorKind::CheckpointRejected.failure_kind(),
            FailureKind::OperationFailed
        );
        assert!(
            !AgentLoopHostErrorKind::CheckpointRejected
                .failure_kind()
                .is_retryable()
        );
        // Generic capacity, configured spend, and context-window exhaustion
        // remain resource-shaped, while truncated output is output-shaped.
        for kind in [
            AgentLoopHostErrorKind::BudgetExceeded,
            AgentLoopHostErrorKind::SpendBudgetExceeded,
            AgentLoopHostErrorKind::ContextOverflow,
        ] {
            assert_eq!(kind.failure_kind(), FailureKind::Resource);
        }
        assert_eq!(
            AgentLoopHostErrorKind::OutputTruncated.failure_kind(),
            FailureKind::OutputTooLarge
        );
    }
}
