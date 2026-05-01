use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityDispatchResult, CapabilityId, ExecutionContext, MountView, Obligation,
    ResourceEstimate, ResourceReservation,
};
use thiserror::Error;

/// Capability-host phase where authorization obligations are satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityObligationPhase {
    Invoke,
    Resume,
    Spawn,
}

/// Request passed to a configured obligation handler before side effects continue.
pub struct CapabilityObligationRequest<'a> {
    pub phase: CapabilityObligationPhase,
    pub context: &'a ExecutionContext,
    pub capability_id: &'a CapabilityId,
    pub estimate: &'a ResourceEstimate,
    pub obligations: &'a [Obligation],
}

/// Effects produced by pre-dispatch obligation handling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityObligationOutcome {
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
}

/// Request passed to a configured obligation handler after successful dispatch.
pub struct CapabilityObligationCompletionRequest<'a> {
    pub phase: CapabilityObligationPhase,
    pub context: &'a ExecutionContext,
    pub capability_id: &'a CapabilityId,
    pub estimate: &'a ResourceEstimate,
    pub obligations: &'a [Obligation],
    pub dispatch: &'a CapabilityDispatchResult,
}

/// Request passed to a configured obligation handler to clean up prepared effects.
pub struct CapabilityObligationAbortRequest<'a> {
    pub phase: CapabilityObligationPhase,
    pub context: &'a ExecutionContext,
    pub capability_id: &'a CapabilityId,
    pub estimate: &'a ResourceEstimate,
    pub obligations: &'a [Obligation],
    pub outcome: &'a CapabilityObligationOutcome,
}

/// Stable, sanitized obligation-handler failure categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityObligationFailureKind {
    Audit,
    Mount,
    Network,
    Output,
    Resource,
    Secret,
}

impl std::fmt::Display for CapabilityObligationFailureKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Audit => "Audit",
            Self::Mount => "Mount",
            Self::Network => "Network",
            Self::Output => "Output",
            Self::Resource => "Resource",
            Self::Secret => "Secret",
        })
    }
}

/// Obligation handler failures. Variants intentionally avoid raw input/output.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CapabilityObligationError {
    #[error("unsupported authorization obligations: {count} item(s)", count = obligations.len())]
    Unsupported { obligations: Vec<Obligation> },
    #[error("authorization obligation failed: {kind}")]
    Failed {
        kind: CapabilityObligationFailureKind,
    },
}

/// Host-provided obligation satisfaction seam.
#[async_trait]
pub trait CapabilityObligationHandler: Send + Sync {
    /// Satisfies all obligations before downstream side effects.
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError>;

    /// Satisfies obligations and returns narrowed dispatch effects.
    async fn prepare(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<CapabilityObligationOutcome, CapabilityObligationError> {
        self.satisfy(request).await?;
        Ok(CapabilityObligationOutcome::default())
    }

    /// Cleans up effects created by [`Self::prepare`] after downstream failure.
    async fn abort(
        &self,
        _request: CapabilityObligationAbortRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        Ok(())
    }

    /// Completes dispatch-result obligations before output returns to callers.
    async fn complete_dispatch(
        &self,
        request: CapabilityObligationCompletionRequest<'_>,
    ) -> Result<CapabilityDispatchResult, CapabilityObligationError> {
        let unsupported = post_dispatch_obligations(request.obligations);
        if unsupported.is_empty() {
            Ok(request.dispatch.clone())
        } else {
            Err(CapabilityObligationError::Unsupported {
                obligations: unsupported,
            })
        }
    }
}

pub(crate) fn post_dispatch_obligations(obligations: &[Obligation]) -> Vec<Obligation> {
    obligations
        .iter()
        .filter(|obligation| {
            matches!(
                obligation,
                Obligation::AuditAfter
                    | Obligation::RedactOutput
                    | Obligation::EnforceResourceCeiling { .. }
                    | Obligation::EnforceOutputLimit { .. }
            )
        })
        .cloned()
        .collect()
}
