//! The auth-resolution dispatch port channel hosts use to resume blocked turns.
//!
//! OAuth callbacks durably resolve an auth flow before handing its exact
//! [`AuthResolved`] value to this port. The port lives below composition so
//! production decorators can add lifecycle activation and compatible-request
//! fanout without introducing a composition dependency.

use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, AuthResolved};
use ironclaw_product_workflow::{ProductAuthTurnGateResumeDispatcher, ProductWorkflowError};
use ironclaw_turns::TurnError;

/// Dispatches one durable terminal auth resolution.
///
/// Implementations MUST be idempotent on `flow_id`: delivery is at least once
/// when the durable delivery marker cannot be recorded after an effect succeeds.
#[async_trait]
pub trait RebornAuthResolutionDispatcher: Send + Sync {
    async fn dispatch_auth_resolved(
        &self,
        resolution: AuthResolved,
    ) -> Result<(), AuthProductError>;
}

#[async_trait]
impl RebornAuthResolutionDispatcher for ProductAuthTurnGateResumeDispatcher {
    async fn dispatch_auth_resolved(
        &self,
        resolution: AuthResolved,
    ) -> Result<(), AuthProductError> {
        ProductAuthTurnGateResumeDispatcher::dispatch_auth_resolved(self, resolution)
            .await
            .map(|_| ())
            .map_err(|error| {
                tracing::debug!(%error, "auth resolution dispatch failed");
                match error {
                    ProductWorkflowError::TurnSubmissionFailed {
                        error: TurnError::Unauthorized,
                    }
                    | ProductWorkflowError::TurnResumeDenied {
                        error: TurnError::Unauthorized,
                    } => AuthProductError::CrossScopeDenied,
                    ProductWorkflowError::TurnSubmissionFailed {
                        error: TurnError::ScopeNotFound,
                    }
                    | ProductWorkflowError::TurnResumeDenied {
                        error: TurnError::ScopeNotFound,
                    } => AuthProductError::UnknownOrExpiredFlow,
                    _ => AuthProductError::BackendUnavailable,
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatcher_contract<T: RebornAuthResolutionDispatcher>() {}

    #[test]
    fn product_dispatcher_implements_the_single_resolution_contract() {
        dispatcher_contract::<ProductAuthTurnGateResumeDispatcher>();
    }
}
