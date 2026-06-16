use ironclaw_authorization::CapabilityLeaseError;
use ironclaw_host_api::{
    CapabilityId, DenyReason, DispatchError, DispatchFailureKind, HostApiError, Obligation,
    RuntimeCredentialAuthRequirement, SecretHandle,
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
            | DispatchError::Mcp { .. }
            | DispatchError::Script { .. }
            | DispatchError::Wasm { .. }
            | DispatchError::FirstParty { .. }) => Self::Dispatch {
                kind: dispatch_error_kind(&other),
                safe_summary: dispatch_error_safe_summary(&other),
            },
        }
    }
}

fn dispatch_error_kind(error: &DispatchError) -> DispatchFailureKind {
    error.failure_kind()
}

fn dispatch_error_safe_summary(error: &DispatchError) -> Option<String> {
    match error {
        DispatchError::FirstParty { safe_summary, .. } => safe_summary.clone(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        ExtensionId, RuntimeCredentialAccountProviderId, RuntimeCredentialAuthRequirement,
        RuntimeDispatchErrorKind, RuntimeKind, SecretHandle,
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
        let kind = dispatch_error_kind(&DispatchError::Mcp {
            kind: RuntimeDispatchErrorKind::Backend,
        });
        assert_eq!(kind.as_str(), "Backend");
    }

    #[test]
    fn dispatch_error_kind_forwards_script_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Script {
            kind: RuntimeDispatchErrorKind::OutputTooLarge,
        });
        assert_eq!(kind.as_str(), "OutputTooLarge");
    }

    #[test]
    fn dispatch_error_kind_forwards_wasm_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Memory,
        });
        assert_eq!(kind.as_str(), "Memory");
    }

    #[test]
    fn dispatch_error_kind_forwards_first_party_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::FirstParty {
            kind: RuntimeDispatchErrorKind::UndeclaredCapability,
            safe_summary: None,
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
            provider: RuntimeCredentialAccountProviderId::new("google").unwrap(),
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
