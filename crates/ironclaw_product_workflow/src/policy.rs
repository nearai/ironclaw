//! Before-inbound policy seam for user-message workflow ingestion.
//!
//! Policies run after protocol authentication and idempotency reservation, but
//! before accepted-message staging. They may allow, rewrite, or reject a
//! user-message payload without exposing raw policy internals to adapters.

use async_trait::async_trait;
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ProductAdapterId,
    ProductInboundEnvelope, ProductRejection, UserMessagePayload,
};

use crate::action::SourceBindingKey;
use crate::error::ProductWorkflowError;

/// Request passed to before-inbound policy implementations.
///
/// This intentionally excludes the full trusted envelope and host-stamped auth
/// claim. Policies see only the user-message payload plus product identity and
/// binding refs needed for policy decisions; the workflow keeps trusted context
/// for downstream staging and turn submission. Policy implementations should use
/// `rate_limit_key` for throttling and must not log raw external refs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeforeInboundPolicyRequest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub source_binding_key: SourceBindingKey,
    /// Stable, workflow-validated partition key for policy-side rate limits.
    pub rate_limit_key: SourceBindingKey,
    pub user_message: UserMessagePayload,
}

impl BeforeInboundPolicyRequest {
    pub fn new(
        envelope: &ProductInboundEnvelope,
        user_message: &UserMessagePayload,
    ) -> Result<Self, ProductWorkflowError> {
        let source_binding_key = SourceBindingKey::new(envelope.source_binding_key())
            .map_err(|reason| ProductWorkflowError::BindingResolutionFailed { reason })?;
        Ok(Self {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            source_binding_key: source_binding_key.clone(),
            rate_limit_key: source_binding_key,
            user_message: user_message.clone(),
        })
    }
}

/// Product-safe policy result for a user message before staging.
#[non_exhaustive]
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
/// fingerprint. The default workflow path wraps policy calls in a bounded
/// `tokio::time::timeout`; production decorators should use `rate_limit_key`
/// for quota decisions and avoid logging raw external refs.
///
/// Returning [`ProductWorkflowError::BeforeInboundPolicyFailed`] with
/// `permanent: true` settles the action as a terminal policy rejection.
/// Returning [`ProductWorkflowError::Transient`] or
/// [`ProductWorkflowError::BeforeInboundPolicyFailed`] with `permanent: false`
/// releases the idempotency fingerprint so the inbound can be retried.
/// Returning [`BeforeInboundPolicyOutcome::Reject`] with a permanent
/// disposition settles the action with a redacted rejection ack; a
/// retryable-disposition reject also releases the fingerprint and lets the
/// adapter re-deliver.
///
/// Multiple production concerns should be composed behind one implementation
/// (for example rate limit → classifier → scope gate). The workflow owns one
/// policy seam so replay-before-policy and settlement semantics stay ordered.
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
