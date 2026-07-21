//! The auth-resolution dispatch port channel hosts use to resume blocked turns.
//!
//! OAuth callbacks durably resolve an auth flow before handing its exact
//! [`AuthResolved`] value to this port. The port lives below composition so
//! production decorators can add lifecycle activation and compatible-request
//! fanout without introducing a composition dependency.

use async_trait::async_trait;
use ironclaw_auth::{AuthProductError, AuthProviderId, AuthResolved, CredentialAccountOwnerScope};
use ironclaw_product_workflow::{ProductAuthTurnGateResumeDispatcher, ProductWorkflowError};
use ironclaw_turns::{IdempotencyKey, TurnError};

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

    /// Notify blocked-run recovery that a non-OAuth provider connection now
    /// satisfies this owner's credential requirement.
    ///
    /// OAuth callers continue to use [`Self::dispatch_auth_resolved`], which
    /// carries their exact flow and gate references. Channel pairing flows use
    /// this narrower operation so they do not fabricate OAuth or credential
    /// account identities. The delivery key must be stable across retries.
    async fn dispatch_provider_connection(
        &self,
        _delivery_key: IdempotencyKey,
        _owner: CredentialAccountOwnerScope,
        _provider: AuthProviderId,
    ) -> Result<(), AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }
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
            .map_err(map_auth_resolution_dispatch_error)
    }
}

fn map_auth_resolution_dispatch_error(error: ProductWorkflowError) -> AuthProductError {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispatcher_contract<T: RebornAuthResolutionDispatcher>() {}

    #[test]
    fn product_dispatcher_implements_the_single_resolution_contract() {
        dispatcher_contract::<ProductAuthTurnGateResumeDispatcher>();
    }

    #[test]
    fn product_dispatch_errors_preserve_security_and_retry_categories() {
        for error in [
            ProductWorkflowError::TurnSubmissionFailed {
                error: TurnError::Unauthorized,
            },
            ProductWorkflowError::TurnResumeDenied {
                error: TurnError::Unauthorized,
            },
        ] {
            assert_eq!(
                map_auth_resolution_dispatch_error(error),
                AuthProductError::CrossScopeDenied
            );
        }
        for error in [
            ProductWorkflowError::TurnSubmissionFailed {
                error: TurnError::ScopeNotFound,
            },
            ProductWorkflowError::TurnResumeDenied {
                error: TurnError::ScopeNotFound,
            },
        ] {
            assert_eq!(
                map_auth_resolution_dispatch_error(error),
                AuthProductError::UnknownOrExpiredFlow
            );
        }
        assert_eq!(
            map_auth_resolution_dispatch_error(ProductWorkflowError::TurnSubmissionFailed {
                error: TurnError::Unavailable {
                    reason: "temporary store outage".to_string(),
                },
            }),
            AuthProductError::BackendUnavailable
        );
    }
}
