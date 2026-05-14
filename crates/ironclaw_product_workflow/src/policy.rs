//! Before-inbound policy seam for user-message workflow ingestion.
//!
//! Policies run after protocol authentication and idempotency reservation, but
//! before accepted-message staging. They may allow, rewrite, or reject a
//! user-message payload without exposing raw policy internals to adapters.

use async_trait::async_trait;
use ironclaw_product_adapters::{ProductInboundEnvelope, ProductRejection, UserMessagePayload};

use crate::error::ProductWorkflowError;

/// Request passed to before-inbound policy implementations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeforeInboundPolicyRequest {
    pub envelope: ProductInboundEnvelope,
    pub user_message: UserMessagePayload,
}

impl BeforeInboundPolicyRequest {
    pub fn new(envelope: &ProductInboundEnvelope, user_message: &UserMessagePayload) -> Self {
        Self {
            envelope: envelope.clone(),
            user_message: user_message.clone(),
        }
    }
}

/// Product-safe policy result for a user message before staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeforeInboundPolicyOutcome {
    /// Continue with the original user-message payload.
    Allow,
    /// Continue with rewritten content/attachments/trigger.
    RewriteUserMessage(UserMessagePayload),
    /// Reject before canonical transcript staging or turn submission.
    Reject(ProductRejection),
}

/// Policy port that runs before user-message staging.
///
/// Implementations must be bounded: the workflow runs `check_user_message`
/// inline on the inbound dispatch path while holding an in-flight idempotency
/// fingerprint, so a slow or stuck policy will stall inbound submissions for
/// the same `(adapter, installation, source binding, external event)` tuple.
/// Callers are responsible for enforcing wall-clock timeouts on any
/// production policy implementation (for example by wrapping the port in a
/// `tokio::time::timeout` decorator).
///
/// Returning a transient [`ProductWorkflowError`] causes the workflow to
/// release the idempotency fingerprint so the inbound can be retried.
/// Returning [`BeforeInboundPolicyOutcome::Reject`] with a permanent
/// disposition settles the action with a redacted rejection ack; a
/// retryable-disposition reject also releases the fingerprint and lets the
/// adapter re-deliver.
#[async_trait]
pub trait BeforeInboundPolicy: Send + Sync {
    async fn check_user_message(
        &self,
        request: BeforeInboundPolicyRequest,
    ) -> Result<BeforeInboundPolicyOutcome, ProductWorkflowError>;
}

/// Backwards-compatible policy used when no production policy is wired.
#[derive(Debug, Clone, Default)]
pub struct NoopBeforeInboundPolicy;

#[async_trait]
impl BeforeInboundPolicy for NoopBeforeInboundPolicy {
    async fn check_user_message(
        &self,
        _request: BeforeInboundPolicyRequest,
    ) -> Result<BeforeInboundPolicyOutcome, ProductWorkflowError> {
        Ok(BeforeInboundPolicyOutcome::Allow)
    }
}
