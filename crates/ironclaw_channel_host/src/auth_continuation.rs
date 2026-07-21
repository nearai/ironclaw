//! The continuation-dispatch port channel hosts use to resume blocked turns.
//!
//! Pairing completions and OAuth callbacks resume `BlockedAuth` runs through
//! the same composed dispatcher; the port lives below composition so a channel
//! host crate can hold `Arc<dyn RebornAuthContinuationDispatcher>` without a
//! composition dependency. The production implementation is
//! [`ProductAuthTurnGateResumeDispatcher`], adapted here.

use async_trait::async_trait;
use ironclaw_auth::{AuthContinuationEvent, AuthProductError};
use ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher;

/// Dispatches a typed continuation event once an OAuth callback flow has
/// completed.
///
/// # Idempotency contract
///
/// Implementations MUST be idempotent on `flow_id`.  The product-auth layer
/// guarantees *at-least-once* delivery: if `dispatch_auth_continuation`
/// succeeds but the subsequent `mark_continuation_dispatched` call fails
/// (e.g. a transient `BackendConflict` or `BackendUnavailable`), the caller
/// will retry the full callback path and dispatch the same `flow_id` again.
/// An implementation that assumes exactly-once delivery will process duplicate
/// continuations and is incorrect.
#[async_trait]
pub trait RebornAuthContinuationDispatcher: Send + Sync {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError>;

    /// Deny a turn gate whose backing auth flow was canceled by lifecycle
    /// cleanup. Non-turn continuations remain cancel-only.
    async fn dispatch_canceled_auth_continuation(
        &self,
        _event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        Err(AuthProductError::BackendUnavailable)
    }
}

#[async_trait]
impl RebornAuthContinuationDispatcher for ProductAuthTurnGateResumeDispatcher {
    async fn dispatch_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        ProductAuthTurnGateResumeDispatcher::dispatch_auth_continuation(self, event).await
    }

    async fn dispatch_canceled_auth_continuation(
        &self,
        event: AuthContinuationEvent,
    ) -> Result<(), AuthProductError> {
        ProductAuthTurnGateResumeDispatcher::dispatch_canceled_auth_continuation(self, event).await
    }
}
