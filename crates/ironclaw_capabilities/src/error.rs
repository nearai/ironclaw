use ironclaw_authorization::CapabilityLeaseError;
use ironclaw_host_api::{
    CapabilityId, DenyReason, DispatchError, DispatchFailureDetail, DispatchFailureKind,
    HostApiError, Obligation, RuntimeCredentialAuthRequirement, SecretHandle,
};
use ironclaw_processes::ProcessError;

use crate::CapabilityObligationFailureKind;
use ironclaw_run_state::{ApprovalStatus, RunStateError, RunStatus};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeContextMismatchKind {
    CapabilityId,
    ApprovalRequestId,
    CapabilityAndApprovalRequestId,
}

/// Capability invocation failures before or during dispatch.
#[derive(Debug, Error)]
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
        required_secrets: Vec<SecretHandle>,
        credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
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
        status: RunStatus,
    },
    #[error("capability {capability} resume context mismatch: {kind:?}")]
    ResumeContextMismatch {
        capability: CapabilityId,
        kind: ResumeContextMismatchKind,
    },
    #[error("lease update failed: {0}")]
    Lease(Box<CapabilityLeaseError>),
    #[error("run-state update failed: {0}")]
    RunState(Box<RunStateError>),
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
        safe_summary: Option<String>,
        detail: Option<DispatchFailureDetail>,
    },
}

impl From<RunStateError> for CapabilityInvocationError {
    fn from(error: RunStateError) -> Self {
        Self::RunState(Box::new(error))
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
                required_secrets,
                credential_requirements,
            } => Self::AuthorizationRequiresAuth {
                capability,
                required_secrets,
                credential_requirements,
            },
            other @ (DispatchError::UnknownCapability { .. }
            | DispatchError::UnknownProvider { .. }
            | DispatchError::RuntimeMismatch { .. }
            | DispatchError::MissingRuntimeBackend { .. }
            | DispatchError::UnsupportedRuntime { .. }
            | DispatchError::MissingAuthorization { .. }
            | DispatchError::AuthorizationExpired { .. }
            | DispatchError::MissingProcessAuthorization { .. }
            | DispatchError::Mcp { .. }
            | DispatchError::Script { .. }
            | DispatchError::Wasm { .. }
            | DispatchError::FirstParty { .. }) => Self::Dispatch {
                kind: dispatch_error_kind(&other),
                safe_summary: dispatch_error_model_visible_cause(&other),
                detail: dispatch_error_detail(&other),
            },
        }
    }
}

fn dispatch_error_kind(error: &DispatchError) -> DispatchFailureKind {
    error.failure_kind()
}

fn dispatch_error_model_visible_cause(error: &DispatchError) -> Option<String> {
    match error {
        DispatchError::Mcp {
            model_visible_cause,
            ..
        }
        | DispatchError::Script {
            model_visible_cause,
            ..
        }
        | DispatchError::Wasm {
            model_visible_cause,
            ..
        } => model_visible_cause.clone(),
        DispatchError::FirstParty { safe_summary, .. } => safe_summary.clone(),
        // These variants carry no free-form runtime string; their `Display`
        // is a stable capability-id + category description that is itself the
        // real cause. Carry it so the model-visible detail channel keeps it
        // (scrubbing of any secret VALUE happens downstream at the
        // Diagnostic-building layer, which lives in a crate that may depend on
        // `ironclaw_turns` — this crate must not).
        DispatchError::UnknownCapability { .. }
        | DispatchError::UnknownProvider { .. }
        | DispatchError::RuntimeMismatch { .. }
        | DispatchError::MissingRuntimeBackend { .. }
        | DispatchError::UnsupportedRuntime { .. }
        | DispatchError::MissingAuthorization { .. }
        | DispatchError::AuthorizationExpired { .. }
        | DispatchError::MissingProcessAuthorization { .. } => Some(error.to_string()),
        // Auth-required carries redacted secret handles; keep it summary-free.
        DispatchError::AuthRequired { .. } => None,
    }
}

fn dispatch_error_detail(error: &DispatchError) -> Option<DispatchFailureDetail> {
    match error {
        DispatchError::FirstParty { detail, .. } => detail.clone(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        DispatchFailureDetail, DispatchInputIssue, DispatchInputIssueCode, ExtensionId,
        RuntimeCredentialAuthRequirement, RuntimeDispatchErrorKind, RuntimeKind, SecretHandle,
        VendorId,
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
    fn dispatch_error_kind_forwards_mcp_runtime_kind_as_str() {
        // Regression (Phase 1): an MCP dispatch error's raw cause must be
        // carried on the model-visible-cause channel — including path/JSON delimiters
        // that the strict summary validator rejects — so it reaches the
        // model-visible Diagnostic/detail downstream instead of being dropped.
        let error = DispatchError::Mcp {
            kind: RuntimeDispatchErrorKind::Backend,
            model_visible_cause: Some("MCP request failed at /tmp/{socket}".to_string()),
        };
        let kind = dispatch_error_kind(&error);
        assert_eq!(kind.as_str(), "Backend");
        assert_eq!(
            dispatch_error_model_visible_cause(&error).as_deref(),
            Some("MCP request failed at /tmp/{socket}")
        );
    }

    #[test]
    fn dispatch_error_kind_forwards_script_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Script {
            kind: RuntimeDispatchErrorKind::OutputTooLarge,
            model_visible_cause: None,
        });
        assert_eq!(kind.as_str(), "OutputTooLarge");
    }

    #[test]
    fn dispatch_error_kind_forwards_wasm_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Memory,
            model_visible_cause: None,
        });
        assert_eq!(kind.as_str(), "Memory");
    }

    #[test]
    fn dispatch_error_kind_forwards_first_party_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::UndeclaredCapability,
            safe_summary: None,
            detail: None,
        });
        assert_eq!(kind.as_str(), "UndeclaredCapability");
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
        let err = CapabilityInvocationError::from(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
            model_visible_cause: None,
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
        let err = CapabilityInvocationError::from(DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::InputEncode,
            safe_summary: Some("trigger_create input failed validation".to_string()),
            detail: Some(DispatchFailureDetail::InvalidInput {
                issues: vec![issue.clone()],
            }),
        });

        match err {
            CapabilityInvocationError::Dispatch { detail, .. } => {
                assert_eq!(
                    detail,
                    Some(DispatchFailureDetail::InvalidInput {
                        issues: vec![issue]
                    })
                );
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
                required_secrets: secrets.clone(),
                credential_requirements: Vec::new(),
            });
            match err {
                CapabilityInvocationError::AuthorizationRequiresAuth {
                    capability,
                    required_secrets,
                    credential_requirements,
                } => {
                    assert_eq!(capability, cap(), "handles: {handles:?}");
                    assert_eq!(required_secrets, secrets, "handles: {handles:?}");
                    assert_eq!(credential_requirements, Vec::new(), "handles: {handles:?}");
                }
                other => panic!("expected AuthorizationRequiresAuth, got {other:?}"),
            }
        }
    }

    #[test]
    fn from_dispatch_auth_required_round_trips_credential_requirements() {
        let requirement = RuntimeCredentialAuthRequirement {
            provider: VendorId::new("google").unwrap(),
            setup: ironclaw_host_api::RuntimeCredentialAccountSetup::OAuth {
                scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".to_string()],
            },
            requester_extension: ExtensionId::new("gmail").unwrap(),
            provider_scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".to_string()],
        };
        let err = CapabilityInvocationError::from(DispatchError::AuthRequired {
            capability: cap(),
            required_secrets: Vec::new(),
            credential_requirements: vec![requirement.clone()],
        });

        match err {
            CapabilityInvocationError::AuthorizationRequiresAuth {
                capability,
                required_secrets,
                credential_requirements,
            } => {
                assert_eq!(capability, cap());
                assert!(required_secrets.is_empty());
                assert_eq!(credential_requirements, vec![requirement]);
            }
            other => panic!("expected AuthorizationRequiresAuth, got {other:?}"),
        }
    }
}
