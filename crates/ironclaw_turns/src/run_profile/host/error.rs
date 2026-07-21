//! Sanitized agent-loop host error type, its kinds/reason-kinds, and the shared
//! `unsupported host method` constructor used by fail-closed port defaults.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{LoopDiagnosticRef, LoopGateRef};

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
    PolicyDenied,
    BudgetExceeded,
    /// The model call would push utilization past the configured pause
    /// threshold. Callers surface an approval gate (foreground or
    /// background) and retry after the user resolves it.
    BudgetApprovalRequired,
    /// Durable budget accounting (reservation read/write/reconcile)
    /// failed. Distinct from `BudgetExceeded`/`BudgetApprovalRequired`
    /// because the failure is in the governor itself, not in the budget
    /// outcome — callers must fail closed.
    BudgetAccountingFailed,
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
            Self::PolicyDenied => "policy_denied",
            Self::BudgetExceeded => "budget_exceeded",
            Self::BudgetApprovalRequired => "budget_approval_required",
            Self::BudgetAccountingFailed => "budget_accounting_failed",
            Self::Unavailable => "unavailable",
            Self::Cancelled => "cancelled",
            Self::CheckpointRejected => "checkpoint_rejected",
            Self::TranscriptWriteFailed => "transcript_write_failed",
            Self::Internal => "internal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopHostErrorReasonKind {
    ModelCreditsExhausted,
    /// A failed model attempt already emitted text through the progress sink.
    /// Retrying it could duplicate externally visible output.
    ModelPartialOutputVisible,
}

impl AgentLoopHostErrorReasonKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelCreditsExhausted => "model_credits_exhausted",
            Self::ModelPartialOutputVisible => "model_partial_output_visible",
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_ref: Option<LoopDiagnosticRef>,
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
            diagnostic_ref: None,
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

    pub fn with_diagnostic_ref(mut self, diagnostic_ref: LoopDiagnosticRef) -> Self {
        self.diagnostic_ref = Some(diagnostic_ref);
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
}
