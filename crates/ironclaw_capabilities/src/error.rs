use ironclaw_authorization::CapabilityLeaseError;
use ironclaw_host_api::{CapabilityId, DenyReason, DispatchError, HostApiError, Obligation};
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
    /// `kind` is a stable, redacted identifier produced by
    /// [`dispatch_error_kind`]. The mapping is part of the public contract:
    /// upstream callers may depend on these strings for routing, metrics, or
    /// audit grouping. The mapping is pinned by unit tests in this crate.
    #[error("dispatch failed: {kind}")]
    Dispatch { kind: String },
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
        Self::Dispatch {
            kind: dispatch_error_kind(&error),
        }
    }
}

fn dispatch_error_kind(error: &DispatchError) -> String {
    match error {
        DispatchError::UnknownCapability { .. } => "UnknownCapability".to_string(),
        DispatchError::UnknownProvider { .. } => "UnknownProvider".to_string(),
        DispatchError::RuntimeMismatch { .. } => "RuntimeMismatch".to_string(),
        DispatchError::MissingRuntimeBackend { .. } => "MissingRuntimeBackend".to_string(),
        DispatchError::UnsupportedRuntime { .. } => "UnsupportedRuntime".to_string(),
        DispatchError::Mcp { kind }
        | DispatchError::Script { kind }
        | DispatchError::Wasm { kind } => kind.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{ExtensionId, RuntimeDispatchErrorKind, RuntimeKind};

    fn cap() -> CapabilityId {
        CapabilityId::new("test.cap").unwrap()
    }

    fn ext() -> ExtensionId {
        ExtensionId::new("test").unwrap()
    }

    #[test]
    fn dispatch_error_kind_maps_unknown_capability_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnknownCapability { capability: cap() });
        assert_eq!(kind, "UnknownCapability");
    }

    #[test]
    fn dispatch_error_kind_maps_unknown_provider_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnknownProvider {
            capability: cap(),
            provider: ext(),
        });
        assert_eq!(kind, "UnknownProvider");
    }

    #[test]
    fn dispatch_error_kind_maps_runtime_mismatch_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::RuntimeMismatch {
            capability: cap(),
            descriptor_runtime: RuntimeKind::Wasm,
            package_runtime: RuntimeKind::Mcp,
        });
        assert_eq!(kind, "RuntimeMismatch");
    }

    #[test]
    fn dispatch_error_kind_maps_missing_runtime_backend_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::MissingRuntimeBackend {
            runtime: RuntimeKind::Wasm,
        });
        assert_eq!(kind, "MissingRuntimeBackend");
    }

    #[test]
    fn dispatch_error_kind_maps_unsupported_runtime_to_stable_literal() {
        let kind = dispatch_error_kind(&DispatchError::UnsupportedRuntime {
            capability: cap(),
            runtime: RuntimeKind::Wasm,
        });
        assert_eq!(kind, "UnsupportedRuntime");
    }

    #[test]
    fn dispatch_error_kind_forwards_mcp_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Mcp {
            kind: RuntimeDispatchErrorKind::Backend,
        });
        assert_eq!(kind, "Backend");
    }

    #[test]
    fn dispatch_error_kind_forwards_script_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Script {
            kind: RuntimeDispatchErrorKind::OutputTooLarge,
        });
        assert_eq!(kind, "OutputTooLarge");
    }

    #[test]
    fn dispatch_error_kind_forwards_wasm_runtime_kind_as_str() {
        let kind = dispatch_error_kind(&DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Memory,
        });
        assert_eq!(kind, "Memory");
    }

    #[test]
    fn from_dispatch_error_flattens_via_dispatch_error_kind() {
        let err =
            CapabilityInvocationError::from(DispatchError::UnknownCapability { capability: cap() });
        match err {
            CapabilityInvocationError::Dispatch { kind } => assert_eq!(kind, "UnknownCapability"),
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }

    #[test]
    fn from_dispatch_error_flattens_redacted_runtime_kind() {
        let err = CapabilityInvocationError::from(DispatchError::Wasm {
            kind: RuntimeDispatchErrorKind::Guest,
        });
        match err {
            CapabilityInvocationError::Dispatch { kind } => assert_eq!(kind, "Guest"),
            other => panic!("expected Dispatch variant, got {other:?}"),
        }
    }
}
