//! Outbound target-resolution vocabulary for the delivery coordinator.
//!
//! The policy-approved render orchestration (`prepare_and_render_product_outbound`)
//! and its request/outcome/error types that once lived here were retired with
//! the `ProductAdapter` contract in P7b (DEL-5). The live outbound path renders
//! through `ChannelAdapter::deliver` driven by
//! [`crate::DeliveryCoordinator`]. What remains is the trusted-metadata type
//! and the lookup-only resolver trait the coordinator resolves reply targets
//! through, plus the shared workflow-error → delivery-failure classifier the
//! coordinator uses.

use crate::{ExternalActorRef, ExternalConversationRef};
use async_trait::async_trait;
use ironclaw_outbound::{DeliveryFailureKind, ValidatedReplyTargetBinding};

use crate::ProductWorkflowError;

/// Product-owned metadata from a trusted conversation-binding lookup for an
/// already validated outbound target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedProductOutboundTargetMetadata {
    pub external_conversation_ref: ExternalConversationRef,
    pub external_actor_ref: Option<ExternalActorRef>,
}

#[async_trait]
pub trait ProductOutboundTargetResolver: Send + Sync {
    /// Resolve already-validated reply-target metadata for rendering.
    ///
    /// Implementations must use a trusted conversation-binding lookup keyed by
    /// the sealed target. They must not choose or substitute a reply target.
    ///
    /// When `require_direct_message` is true, implementations must return
    /// [`ProductWorkflowError::OutboundTargetNotDirectMessage`] if the resolved
    /// target is not a personal direct message.
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
        require_direct_message: bool,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError>;
}

/// Classify a workflow error into the delivery-failure kind the coordinator
/// records when reply-target resolution fails before render.
pub(crate) fn delivery_failure_kind_for_workflow_error(
    error: &ProductWorkflowError,
) -> DeliveryFailureKind {
    match error {
        ProductWorkflowError::Transient { .. } => DeliveryFailureKind::TransportUnavailable,
        ProductWorkflowError::BindingAccessDenied
        | ProductWorkflowError::BindingRequired { .. }
        | ProductWorkflowError::UnknownInstallation
        | ProductWorkflowError::InvalidBindingRequest { .. }
        | ProductWorkflowError::OutboundTargetNotDirectMessage => DeliveryFailureKind::Rejected,
        _ => DeliveryFailureKind::Unknown,
    }
}
