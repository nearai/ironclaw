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
