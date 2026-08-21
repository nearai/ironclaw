use std::fmt;

use ironclaw_authorization::CapabilityLeaseError;
use ironclaw_host_api::{
    decision::{DenyReason, Obligation},
    dispatch::{
        DispatchAuthRequirement, DispatchError, DispatchFailureDetail, DispatchFailureKind,
        ProviderDiagnostic, provider_diagnostic_model_cause,
    },
    error::HostApiError,
    ids::CapabilityId,
};
use ironclaw_processes::{ProcessError, ProcessInvocationError, ProcessInvocationStatus};

use crate::CapabilityObligationFailureKind;
use ironclaw_approvals::{ApprovalStatus, ApprovalStoreError};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeContextMismatchKind {
    CapabilityId,
    ApprovalRequestId,
    CapabilityAndApprovalRequestId,
}

/// Capability invocation failures before or during dispatch.
#[derive(Error)]
pub enum CapabilityInvocationError {
    #[error("unknown capability {capability}")]
    UnknownCapability { capability: CapabilityId },
    #[error("capability {capability} invocation denied: {reason:?}")]
    AuthorizationDenied {
        capability: CapabilityId,
        reason: DenyReason,
        /// Optional model-visible sanitized cause behind the collapsed
        /// [`DenyReason`] (e.g. the runtime-policy planner's "requires process
        /// effects but policy resolves to `ProcessBackendKind::None`"). The
        /// closed `DenyReason` set cannot carry it, so callers that resolve a
        /// specific fail-closed reason thread it here; `None` when the bare
        /// verdict is self-explanatory. Surfaced via [`sanitized_failure_message`].
        detail: Option<String>,
    },
    #[error("capability {capability} returned unsupported authorization obligations")]
    UnsupportedObligations {
        capability: CapabilityId,
        obligations: Vec<Obligation>,
    },
    #[error("capability {capability} obligation handling failed: {kind}")]
    ObligationFailed {
        capability: CapabilityId,
        kind: CapabilityObligationFailureKind,
    },
    #[error("capability {capability} invocation requires approval")]
    AuthorizationRequiresApproval { capability: CapabilityId },
    #[error("capability {capability} invocation requires authentication")]
    AuthorizationRequiresAuth {
        capability: CapabilityId,
        requirement: Box<DispatchAuthRequirement>,
    },
    #[error("capability {capability} invocation fingerprint failed: {source}")]
    InvocationFingerprint {
        capability: CapabilityId,
        source: HostApiError,
    },
    #[error("capability {capability} approval request does not match invocation: {field}")]
    ApprovalRequestMismatch {
        capability: CapabilityId,
        field: &'static str,
    },
    #[error("capability {capability} approval fingerprint mismatch")]
    ApprovalFingerprintMismatch { capability: CapabilityId },
    #[error("capability {capability} approval is not approved: {status:?}")]
    ApprovalNotApproved {
        capability: CapabilityId,
        status: ApprovalStatus,
    },
    #[error("capability {capability} approval path requires {store}")]
    ApprovalStoreMissing {
        capability: CapabilityId,
        store: &'static str,
    },
    #[error("capability {capability} approval lease is missing")]
    ApprovalLeaseMissing { capability: CapabilityId },
    #[error("capability {capability} resume requires {store}")]
    ResumeStoreMissing {
        capability: CapabilityId,
        store: &'static str,
    },
    #[error("capability {capability} spawn requires a process manager")]
    ProcessManagerMissing { capability: CapabilityId },
    #[error("capability {capability} cannot resume from run status {status:?}")]
    ResumeNotBlocked {
        capability: CapabilityId,
        status: ProcessInvocationStatus,
    },
    #[error("capability {capability} resume context mismatch: {kind:?}")]
    ResumeContextMismatch {
        capability: CapabilityId,
        kind: ResumeContextMismatchKind,
    },
    #[error("lease update failed: {0}")]
    Lease(Box<CapabilityLeaseError>),
    #[error("approval store update failed: {0}")]
    ApprovalStore(Box<ApprovalStoreError>),
    #[error("process invocation update failed: {0}")]
    InvocationState(Box<ProcessInvocationError>),
    #[error("process update failed: {0}")]
    Process(Box<ProcessError>),
    /// Runtime dispatch failure surfaced through the neutral host API port.
    ///
    /// `kind` is a stable, redacted category. Its display string remains part
    /// of the public contract for routing, metrics, and audit grouping, but
    /// callers that stay in-process can keep typed failure identity.
    #[error("dispatch failed: {kind}")]
    Dispatch {
        kind: DispatchFailureKind,
        /// Provider-authored metadata remains typed and Debug-redacted until
        /// the runtime's model-diagnostic scrub/fence seam.
        provider_diagnostic: Option<Box<ProviderDiagnostic>>,
        /// Candidate public label — persisted, published, and rendered by
        /// product surfaces (the chat tool-failure card reads this field).
        /// Three-tier precedence, most-trusted first:
        ///
        /// 1. A host-authored summary (`DispatchFailureDetail::HostSummary`,
        ///    set via `DispatchError::with_host_summary`), when present —
        ///    e.g. a first-party capability's fixed rejection summary.
        /// 2. Otherwise the provider diagnostic's text (formatted
        ///    `code`/`message`, or a bare `message`), carried forward
        ///    deliberately — not accidentally — so the user still sees the
        ///    real vendor reason ("token lacks repo scope") instead of a
        ///    generic sentence. This is validation-gated downstream: only
        ///    summaries that pass the strict `LoopSafeSummary` gate (bounded,
        ///    no control chars, no delimiters, no credential markers)
        ///    reach the public surface; anything that fails validation
        ///    degrades to the kind's fixed sentence there.
        /// 3. `None` when neither is available, which the downstream gate
        ///    also renders as the kind's fixed sentence
        ///    (`kind.human_summary()`).
        ///
        /// Debug-redacted here too (see `provider_diagnostic`'s doc) even
        /// though it is a plain `String`, so Debug output never depends on
        /// which downstream seam scrubs first.
        safe_summary: Option<String>,
        detail: Option<DispatchFailureDetail>,
    },
}

impl fmt::Debug for CapabilityInvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCapability { capability } => f
                .debug_struct("UnknownCapability")
                .field("capability", capability)
                .finish(),
            Self::AuthorizationDenied {
                capability,
                reason,
                detail,
            } => f
                .debug_struct("AuthorizationDenied")
                .field("capability", capability)
                .field("reason", reason)
                .field("detail", detail)
                .finish(),
            Self::UnsupportedObligations {
                capability,
                obligations,
            } => f
                .debug_struct("UnsupportedObligations")
                .field("capability", capability)
                .field("obligations", obligations)
                .finish(),
            Self::ObligationFailed { capability, kind } => f
                .debug_struct("ObligationFailed")
                .field("capability", capability)
                .field("kind", kind)
                .finish(),
            Self::AuthorizationRequiresApproval { capability } => f
                .debug_struct("AuthorizationRequiresApproval")
                .field("capability", capability)
                .finish(),
            Self::AuthorizationRequiresAuth {
                capability,
                requirement,
            } => f
                .debug_struct("AuthorizationRequiresAuth")
                .field("capability", capability)
                .field(
                    "required_secrets",
                    &format!(
                        "[{} handle(s) redacted]",
                        requirement.required_secrets.len()
                    ),
                )
                .field(
                    "credential_requirements",
                    &format!(
                        "[{} requirement(s) redacted]",
                        requirement.credential_requirements.len()
                    ),
                )
                .field("model_visible_cause", &requirement.model_visible_cause)
                .finish(),
            Self::InvocationFingerprint { capability, source } => f
                .debug_struct("InvocationFingerprint")
                .field("capability", capability)
                .field("source", source)
                .finish(),
            Self::ApprovalRequestMismatch { capability, field } => f
                .debug_struct("ApprovalRequestMismatch")
                .field("capability", capability)
                .field("field", field)
                .finish(),
            Self::ApprovalFingerprintMismatch { capability } => f
                .debug_struct("ApprovalFingerprintMismatch")
                .field("capability", capability)
                .finish(),
            Self::ApprovalNotApproved { capability, status } => f
                .debug_struct("ApprovalNotApproved")
                .field("capability", capability)
                .field("status", status)
                .finish(),
            Self::ApprovalStoreMissing { capability, store } => f
                .debug_struct("ApprovalStoreMissing")
                .field("capability", capability)
                .field("store", store)
                .finish(),
            Self::ApprovalLeaseMissing { capability } => f
                .debug_struct("ApprovalLeaseMissing")
                .field("capability", capability)
                .finish(),
            Self::ResumeStoreMissing { capability, store } => f
                .debug_struct("ResumeStoreMissing")
                .field("capability", capability)
                .field("store", store)
                .finish(),
            Self::ProcessManagerMissing { capability } => f
                .debug_struct("ProcessManagerMissing")
                .field("capability", capability)
                .finish(),
            Self::ResumeNotBlocked { capability, status } => f
                .debug_struct("ResumeNotBlocked")
                .field("capability", capability)
                .field("status", status)
                .finish(),
            Self::ResumeContextMismatch { capability, kind } => f
                .debug_struct("ResumeContextMismatch")
                .field("capability", capability)
                .field("kind", kind)
                .finish(),
            Self::Lease(source) => f.debug_tuple("Lease").field(source).finish(),
            Self::ApprovalStore(source) => f.debug_tuple("ApprovalStore").field(source).finish(),
            Self::InvocationState(source) => {
                f.debug_tuple("InvocationState").field(source).finish()
            }
            Self::Process(source) => f.debug_tuple("Process").field(source).finish(),
            // `safe_summary` carries the same untrusted provider text as
            // `provider_diagnostic` (see `From<DispatchError>`), so it is
            // redacted here too — never log or render it directly.
            Self::Dispatch {
                kind,
                provider_diagnostic,
                detail,
                ..
            } => f
                .debug_struct("Dispatch")
                .field("kind", kind)
                .field("provider_diagnostic", provider_diagnostic)
                .field("safe_summary", &"<redacted>")
                // `DispatchFailureDetail`'s Debug implementation selectively
                // preserves structured input issues while redacting raw
                // provider/backend causes.
                .field("detail", detail)
                .finish(),
        }
    }
}

impl From<ApprovalStoreError> for CapabilityInvocationError {
    fn from(error: ApprovalStoreError) -> Self {
        Self::ApprovalStore(Box::new(error))
    }
}

impl From<ProcessInvocationError> for CapabilityInvocationError {
    fn from(error: ProcessInvocationError) -> Self {
        Self::InvocationState(Box::new(error))
    }
}

impl From<ProcessError> for CapabilityInvocationError {
    fn from(error: ProcessError) -> Self {
        Self::Process(Box::new(error))
    }
}

impl From<DispatchError> for CapabilityInvocationError {
    fn from(error: DispatchError) -> Self {
        match error {
            DispatchError::AuthRequired {
                capability,
                requirement,
            } => Self::AuthorizationRequiresAuth {
                capability,
                requirement,
            },
            DispatchError::Rejected {
                kind,
                diagnostic,
                detail,
                ..
            } => {
                // Three-tier precedence (see the `safe_summary` field doc):
                // 1. A host-authored `DispatchFailureDetail::HostSummary`
                //    wins outright — it is the caller's own trusted label.
                // 2. Otherwise the vendor/provider diagnostic's text is
                //    carried forward deliberately, so the user still sees
                //    the real reason instead of a generic sentence. A
                //    structured `code` (e.g. a WASM guest's stable taxonomy
                //    code) uses the formatted `code`/`message` combination so
                //    the code always surfaces; a bare `message` with no
                //    `code` rides through unprefixed. This text still fails
                //    closed downstream through the strict `LoopSafeSummary`
                //    gate (`dispatch_failure_message`), which degrades
                //    anything that doesn't validate to the kind's fixed
                //    sentence — this layer only carries the candidate text
                //    forward.
                // 3. `None` when neither is available; the same downstream
                //    gate renders the kind's fixed sentence.
                let (safe_summary, detail) = match detail {
                    Some(DispatchFailureDetail::HostSummary { summary, detail }) => {
                        (Some(summary.into_inner()), detail.map(|detail| *detail))
                    }
                    detail => {
                        let summary = diagnostic.as_ref().and_then(|diagnostic| {
                            if diagnostic.code.is_some() {
                                provider_diagnostic_model_cause(diagnostic)
                            } else {
                                diagnostic
                                    .message
                                    .as_ref()
                                    .map(|message| message.as_str().to_string())
                            }
                        });
                        (summary, detail)
                    }
                };
                Self::Dispatch {
                    kind,
                    provider_diagnostic: diagnostic,
                    safe_summary,
                    detail,
                }
            }
            other @ (DispatchError::UnknownCapability { .. }
            | DispatchError::UnknownProvider { .. }
            | DispatchError::RuntimeMismatch { .. }
            | DispatchError::MissingRuntimeBackend { .. }
            | DispatchError::UnsupportedRuntime { .. }
            | DispatchError::MissingAuthorization { .. }
            | DispatchError::AuthorizationExpired { .. }
            | DispatchError::MissingProcessAuthorization { .. }) => Self::Dispatch {
                kind: dispatch_error_kind(&other),
                provider_diagnostic: None,
                safe_summary: dispatch_error_model_visible_cause(&other),
                detail: None,
            },
        }
    }
}

fn dispatch_error_kind(error: &DispatchError) -> DispatchFailureKind {
    error.failure_kind()
}

/// These variants carry no free-form runtime string; their `Display` is a
/// stable capability-id + category description that is itself the real
/// cause. Carry it so the model-visible detail channel keeps it (scrubbing
/// of any secret VALUE happens downstream at the Diagnostic-building layer,
/// which lives in a crate that may depend on `ironclaw_turns` — this crate
/// must not).
fn dispatch_error_model_visible_cause(error: &DispatchError) -> Option<String> {
    match error {
        DispatchError::UnknownCapability { .. }
        | DispatchError::UnknownProvider { .. }
        | DispatchError::RuntimeMismatch { .. }
        | DispatchError::MissingRuntimeBackend { .. }
        | DispatchError::UnsupportedRuntime { .. }
        | DispatchError::MissingAuthorization { .. }
        | DispatchError::AuthorizationExpired { .. }
        | DispatchError::MissingProcessAuthorization { .. } => Some(error.to_string()),
        // Auth-required carries redacted secret handles; keep it summary-free.
        // Rejected's cause rides the typed `provider_diagnostic` channel
        // instead of this raw-string one (see the `Rejected` arm above).
        DispatchError::AuthRequired { .. } | DispatchError::Rejected { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        decision::RuntimeCredentialAuthRequirement,
        dispatch::{
            DispatchAuthRequirement, DispatchFailureDetail, DispatchInputIssue,
            DispatchInputIssueCode, RuntimeDispatchErrorKind,
        },
        ids::{ExtensionId, SecretHandle, VendorId},
        runtime::RuntimeKind,
        safe_summary::SafeSummary,
    };

    fn cap() -> CapabilityId {
        CapabilityId::new("test.cap").unwrap()
    }

    fn ext() -> ExtensionId {
        ExtensionId::new("test").unwrap()
    }

    #[test]
    fn dispatch_error_kind_maps_unknown_capability_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnknownCapability { capability: cap() });
        assert_eq!(kind.as_str(), "UnknownCapability");
    }

    #[test]
    fn dispatch_error_kind_maps_unknown_provider_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnknownProvider {
            capability: cap(),
            provider: ext(),
        });
        assert_eq!(kind.as_str(), "UnknownProvider");
    }

    #[test]
    fn dispatch_error_kind_maps_runtime_mismatch_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::RuntimeMismatch {
            capability: cap(),
            descriptor_runtime: RuntimeKind::Wasm,
            package_runtime: RuntimeKind::Mcp,
        });
        assert_eq!(kind.as_str(), "RuntimeMismatch");
    }

    #[test]
    fn dispatch_error_kind_maps_missing_runtime_backend_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Wasm,
        });
        assert_eq!(kind.as_str(), "MissingRuntimeBackend");
    }

    #[test]
    fn dispatch_error_kind_maps_unsupported_runtime_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnsupportedRuntime {
            capability: cap(),
            runtime: RuntimeKind::Wasm,
        });
        assert_eq!(kind.as_str(), "UnsupportedRuntime");
    }

    #[test]
    fn dispatch_error_kind_forwards_runtime_kind_via_rejected() {
        // Regression (Phase 1): a runtime dispatch error's raw cause must be
        // carried on the typed diagnostic channel — including path/JSON delimiters
        // that the strict summary validator rejects — so it reaches the
        // model-visible Diagnostic/detail downstream instead of being dropped.
        let error = DispatchError::Rejected {
            runtime: Some(RuntimeKind::Mcp),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Backend),
            diagnostic: Some(Box::new(ProviderDiagnostic {
                code: None,
                message: Some(ironclaw_host_api::dispatch::UntrustedProviderMessage::new(
                    "MCP request failed at /tmp/{socket}",
                )),
                retry_after: None,
            })),
            detail: None,
        };
        let kind = dispatch_error_kind(&error);
        assert_eq!(kind.as_str(), "Backend");
        let DispatchError::Rejected {
            diagnostic: Some(diagnostic),
            ..
        } = &error
        else {
            panic!("expected Rejected variant");
        };
        assert_eq!(
            provider_diagnostic_model_cause(diagnostic.as_ref()).as_deref(),
            Some("provider message: MCP request failed at /tmp/{socket}")
        );
    }

    #[test]
    fn provider_rejection_preserves_typed_diagnostic_for_model_projection() {
        let error = CapabilityInvocationError::from(DispatchError::Rejected {
            runtime: Some(RuntimeKind::Mcp),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Client),
            diagnostic: Some(Box::new(ProviderDiagnostic {
                code: Some(ironclaw_host_api::dispatch::ProviderErrorCode::new(
                    "mcp_tool_rejected",
                )),
                message: Some(ironclaw_host_api::dispatch::UntrustedProviderMessage::new(
                    "token lacks repo scope",
                )),
                retry_after: None,
            })),
            detail: None,
        });
        assert!(!format!("{error:?}").contains("token lacks repo scope"));

        let CapabilityInvocationError::Dispatch {
            kind,
            provider_diagnostic: Some(diagnostic),
            safe_summary,
            ..
        } = error
        else {
            panic!("provider rejection must remain a recoverable dispatch failure");
        };
        assert_eq!(
            kind,
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Client)
        );
        // Tier 2 (no host summary attached): the formatted vendor cause is
        // the candidate public label too — it still fails closed downstream
        // through the strict `LoopSafeSummary` gate (`dispatch_failure_message`
        // in `ironclaw_host_runtime`), which degrades anything unsafe to the
        // kind's fixed sentence; this layer only carries the candidate text
        // forward so the user still sees the real reason.
        assert_eq!(
            safe_summary.as_deref(),
            Some(
                "provider error code: mcp_tool_rejected; provider message: token lacks repo scope"
            )
        );
        assert_eq!(
            provider_diagnostic_model_cause(&diagnostic).as_deref(),
            Some(
                "provider error code: mcp_tool_rejected; provider message: token lacks repo scope"
            )
        );
    }

    #[test]
    fn from_dispatch_error_preserves_top_level_dispatch_kind() {
        let err =
            CapabilityInvocationError::from(DispatchError::UnknownCapability { capability: cap() });
        match err {
            CapabilityInvocationError::Dispatch { kind, .. } => {
                assert_eq!(kind, DispatchFailureKind::UnknownCapability)
            }
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }

    #[test]
    fn from_dispatch_error_preserves_redacted_runtime_kind() {
        let err = CapabilityInvocationError::from(DispatchError::Rejected {
            runtime: Some(RuntimeKind::Wasm),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Guest),
            diagnostic: None,
            detail: None,
        });
        match err {
            CapabilityInvocationError::Dispatch { kind, .. } => {
                assert_eq!(
                    kind,
                    DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Guest)
                )
            }
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }

    #[test]
    fn from_dispatch_error_preserves_first_party_detail() {
        let issue =
            DispatchInputIssue::new("schedule.kind", DispatchInputIssueCode::MissingRequired)
                .expected("cron or once");
        let err = CapabilityInvocationError::from(DispatchError::Rejected {
            runtime: Some(RuntimeKind::FirstParty),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::InputEncode),
            diagnostic: Some(Box::new(ProviderDiagnostic {
                code: None,
                message: Some(ironclaw_host_api::dispatch::UntrustedProviderMessage::new(
                    "trigger_create input failed validation",
                )),
                retry_after: None,
            })),
            detail: Some(DispatchFailureDetail::InvalidInput {
                issues: vec![issue.clone()],
            }),
        });

        match err {
            CapabilityInvocationError::Dispatch {
                detail,
                safe_summary,
                ..
            } => {
                assert_eq!(
                    detail,
                    Some(DispatchFailureDetail::InvalidInput {
                        issues: vec![issue]
                    })
                );
                // Tier 2: `detail` here is `InvalidInput`, not a host
                // summary, so `safe_summary` falls through to the vendor
                // diagnostic. `code: None` rides the bare-message branch
                // unprefixed — no "provider error code:"/"provider message:"
                // label.
                assert_eq!(
                    safe_summary.as_deref(),
                    Some("trigger_create input failed validation")
                );
            }
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }

    #[test]
    fn capability_invocation_error_dispatch_debug_redacts_diagnostic_detail_text() {
        // `DispatchFailureDetail::Diagnostic { text }` carries an untrusted
        // raw provider/backend cause (never-log content); the folded
        // `Dispatch` variant's Debug must not print it, mirroring
        // `DispatchError::Rejected`'s Debug redaction.
        let err = CapabilityInvocationError::from(DispatchError::Rejected {
            runtime: Some(RuntimeKind::FirstParty),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::OperationFailed),
            diagnostic: None,
            detail: Some(DispatchFailureDetail::Diagnostic {
                text: "leak-me-not: /secret/path token=abc123".to_string(),
            }),
        });

        let debug_output = format!("{err:?}");
        assert!(!debug_output.contains("leak-me-not"));
        assert!(!debug_output.contains("/secret/path"));
        assert!(!debug_output.contains("abc123"));
    }

    #[test]
    fn capability_invocation_error_dispatch_debug_keeps_structured_input_detail_visible() {
        let err = CapabilityInvocationError::from(DispatchError::Rejected {
            runtime: Some(RuntimeKind::FirstParty),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::InputEncode),
            diagnostic: None,
            detail: Some(DispatchFailureDetail::InvalidInput {
                issues: vec![DispatchInputIssue::new(
                    "schedule.kind",
                    DispatchInputIssueCode::MissingRequired,
                )],
            }),
        });

        let debug_output = format!("{err:?}");
        assert!(debug_output.contains("InvalidInput"));
        assert!(debug_output.contains("schedule.kind"));
    }

    /// Invariant pin, tier 1: a `Rejected` carrying BOTH a host-authored
    /// summary (`DispatchFailureDetail::HostSummary`) and a vendor-authored
    /// cause keeps the host summary as the public `safe_summary` — it must
    /// never be silently dropped in favor of the vendor cause (the defect
    /// `registry.rs`'s old `model_visible_cause.or(safe_summary)` had) — while
    /// the vendor text still rides `provider_diagnostic` for the
    /// model-visible channel. The two channels are never merged into one.
    #[test]
    fn rejected_host_summary_wins_over_vendor_diagnostic() {
        let vendor_cause = "vendor backend returned 503 at /internal/route";
        let host_summary = "the tool's backend failed";
        let error = DispatchError::provider_rejected(
            Some(RuntimeKind::FirstParty),
            DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Backend),
            Some(vendor_cause.to_string()),
            Some(DispatchFailureDetail::HostSummary {
                summary: SafeSummary::new(host_summary).unwrap(),
                detail: Some(Box::new(DispatchFailureDetail::InvalidInput {
                    issues: vec![DispatchInputIssue::new(
                        "schedule.kind",
                        DispatchInputIssueCode::TypeMismatch,
                    )],
                })),
            }),
        );
        let err = CapabilityInvocationError::from(error);

        match err {
            CapabilityInvocationError::Dispatch {
                safe_summary,
                provider_diagnostic,
                detail,
                ..
            } => {
                assert_eq!(safe_summary.as_deref(), Some(host_summary));
                let diagnostic = provider_diagnostic.expect("vendor cause must ride diagnostic");
                assert_eq!(
                    provider_diagnostic_model_cause(&diagnostic).as_deref(),
                    Some(format!("provider message: {vendor_cause}").as_str())
                );
                assert!(matches!(
                    detail,
                    Some(DispatchFailureDetail::InvalidInput { issues })
                        if issues.len() == 1 && issues[0].path == "schedule.kind"
                ));
            }
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }

    /// Invariant pin, tier 2: no host summary attached — the vendor
    /// diagnostic's text becomes the public `safe_summary` (carried forward
    /// deliberately, still validation-gated downstream by the strict
    /// `LoopSafeSummary` gate in `ironclaw_host_runtime`) rather than
    /// dropping to `None`. This is the same text that also rides
    /// `provider_diagnostic` for the model-visible channel.
    #[test]
    fn rejected_diagnostic_without_host_summary_becomes_the_public_label() {
        let err = CapabilityInvocationError::from(DispatchError::Rejected {
            runtime: Some(RuntimeKind::Mcp),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Backend),
            diagnostic: Some(Box::new(ProviderDiagnostic {
                code: None,
                message: Some(ironclaw_host_api::dispatch::UntrustedProviderMessage::new(
                    "backend exploded",
                )),
                retry_after: None,
            })),
            detail: None,
        });

        match err {
            CapabilityInvocationError::Dispatch {
                safe_summary,
                provider_diagnostic,
                ..
            } => {
                assert_eq!(safe_summary.as_deref(), Some("backend exploded"));
                assert!(provider_diagnostic.is_some());
            }
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }

    /// Invariant pin, tier 3: neither a host summary nor a vendor diagnostic
    /// is present — `safe_summary` is `None`, and the downstream strict
    /// `LoopSafeSummary` gate renders the kind's fixed sentence
    /// (`kind.human_summary()`) instead.
    #[test]
    fn rejected_without_host_summary_or_diagnostic_yields_none_safe_summary() {
        let err = CapabilityInvocationError::from(DispatchError::Rejected {
            runtime: Some(RuntimeKind::Wasm),
            kind: DispatchFailureKind::Runtime(RuntimeDispatchErrorKind::Guest),
            diagnostic: None,
            detail: None,
        });

        match err {
            CapabilityInvocationError::Dispatch {
                safe_summary,
                provider_diagnostic,
                ..
            } => {
                assert_eq!(safe_summary, None);
                assert!(provider_diagnostic.is_none());
            }
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }

    #[test]
    fn from_dispatch_auth_required_round_trips_required_secrets() {
        let cases: &[&[&str]] = &[
            &[],
            &["google-access-token"],
            &["google-access-token", "google-refresh-token"],
        ];
        for handles in cases {
            let secrets: Vec<SecretHandle> = handles
                .iter()
                .map(|h| SecretHandle::new(*h).unwrap())
                .collect();
            let err = CapabilityInvocationError::from(DispatchError::AuthRequired {
                capability: cap(),
                requirement: Box::new(DispatchAuthRequirement {
                    required_secrets: secrets.clone(),
                    credential_requirements: Vec::new(),
                    model_visible_cause: None,
                }),
            });
            match err {
                CapabilityInvocationError::AuthorizationRequiresAuth {
                    capability,
                    requirement,
                    ..
                } => {
                    assert_eq!(capability, cap(), "handles: {handles:?}");
                    assert_eq!(
                        requirement.required_secrets, secrets,
                        "handles: {handles:?}"
                    );
                    assert_eq!(
                        requirement.credential_requirements,
                        Vec::new(),
                        "handles: {handles:?}"
                    );
                }
                other => panic!("expected AuthorizationRequiresAuth, got {other:?}"),
            }
        }
    }

    #[test]
    fn from_dispatch_auth_required_round_trips_credential_requirements() {
        let credential_requirement = RuntimeCredentialAuthRequirement {
            provider: VendorId::new("google").unwrap(),
            setup: ironclaw_host_api::capability::RuntimeCredentialAccountSetup::OAuth {
                scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".to_string()],
            },
            requester_extension: ExtensionId::new("gmail").unwrap(),
            provider_scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".to_string()],
        };
        let err = CapabilityInvocationError::from(DispatchError::AuthRequired {
            capability: cap(),
            requirement: Box::new(DispatchAuthRequirement {
                required_secrets: Vec::new(),
                credential_requirements: vec![credential_requirement.clone()],
                model_visible_cause: Some(ProviderDiagnostic {
                    code: None,
                    message: Some(ironclaw_host_api::dispatch::UntrustedProviderMessage::new(
                        "Bad credentials",
                    )),
                    retry_after: None,
                }),
            }),
        });

        match err {
            CapabilityInvocationError::AuthorizationRequiresAuth {
                capability,
                requirement,
            } => {
                assert_eq!(capability, cap());
                assert!(requirement.required_secrets.is_empty());
                assert_eq!(
                    requirement.credential_requirements,
                    vec![credential_requirement]
                );
                assert_eq!(
                    requirement
                        .model_visible_cause
                        .as_ref()
                        .and_then(|diagnostic| diagnostic.message.as_ref())
                        .map(|message| message.as_str()),
                    Some("Bad credentials")
                );
                assert!(
                    !format!("{:?}", requirement.model_visible_cause).contains("Bad credentials")
                );
            }
            other => panic!("expected AuthorizationRequiresAuth, got {other:?}"),
        }
    }
}
