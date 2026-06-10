//! Product outbound orchestration after outbound policy approval.

use async_trait::async_trait;
use ironclaw_outbound::{
    CommunicationPreferenceRepository, DeliveryFailureKind, OutboundDeliveryAttempt,
    OutboundDeliveryDecision, OutboundDeliveryStatus, OutboundError, OutboundPolicyService,
    OutboundPushKind, PrepareCommunicationDeliveryRequest, UpdateDeliveryStatusRequest,
    ValidatedReplyTargetBinding,
};
use ironclaw_product_adapters::{
    ExternalActorRef, ExternalConversationRef, OutboundDeliverySink, ProductAdapter,
    ProductAdapterError, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProductRenderOutcome, ProjectionCursor, ProtocolHttpEgress,
};
use thiserror::Error;
use tracing::debug;

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
    async fn resolve_product_outbound_target_metadata(
        &self,
        target: &ValidatedReplyTargetBinding,
    ) -> Result<VerifiedProductOutboundTargetMetadata, ProductWorkflowError>;
}

/// Inputs needed to validate and render one product outbound delivery.
pub struct ProductOutboundDeliveryRequest<'a> {
    pub delivery: PrepareCommunicationDeliveryRequest,
    pub payload: ProductOutboundPayload,
    pub projection_cursor: ProjectionCursor,
    pub adapter: &'a dyn ProductAdapter,
    pub egress: &'a dyn ProtocolHttpEgress,
    pub delivery_sink: &'a dyn OutboundDeliverySink,
}

/// Non-fatal failure while recording a post-render delivery status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductOutboundStatusUpdateFailure {
    Backend,
    Serialization,
    InvalidRequest,
    ScopeMismatch,
    DeliveryNotFound,
    AccessDenied,
    CasConflict,
}

/// Result of the policy-approved product outbound orchestration step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductOutboundDeliveryOutcome {
    NoDelivery,
    Rejected {
        attempt: OutboundDeliveryAttempt,
    },
    Rendered {
        attempt: OutboundDeliveryAttempt,
        render_outcome: ProductRenderOutcome,
    },
    RenderedStatusUpdateFailed {
        attempt: OutboundDeliveryAttempt,
        render_outcome: ProductRenderOutcome,
        status_update_error: ProductOutboundStatusUpdateFailure,
    },
}

/// Error returned before or during product outbound rendering.
#[derive(Debug, Error)]
pub enum ProductOutboundDeliveryError {
    #[error("outbound policy failed: {0}")]
    Outbound(#[from] OutboundError),
    #[error("product workflow failed: {source}")]
    Workflow {
        source: ProductWorkflowError,
        status_update_error: Option<ProductOutboundStatusUpdateFailure>,
    },
    #[error("product adapter failed: {source}")]
    Adapter {
        source: ProductAdapterError,
        status_update_error: Option<ProductOutboundStatusUpdateFailure>,
    },
    #[error("product outbound payload cannot satisfy {delivery_kind:?}")]
    PayloadKindMismatch {
        delivery_kind: OutboundPushKind,
        payload_kind: Option<OutboundPushKind>,
        status_update_error: Option<ProductOutboundStatusUpdateFailure>,
    },
}

/// Resolve, validate, and render one outbound product delivery.
///
/// Outbound policy remains the authority for candidate validation and attempt
/// metadata. Product workflow only attaches product-target metadata after the
/// policy service mints a validated reply target.
pub async fn prepare_and_render_product_outbound(
    outbound_policy: &OutboundPolicyService<'_>,
    communication_preferences: &dyn CommunicationPreferenceRepository,
    target_resolver: &dyn ProductOutboundTargetResolver,
    request: ProductOutboundDeliveryRequest<'_>,
) -> Result<ProductOutboundDeliveryOutcome, ProductOutboundDeliveryError> {
    let delivery_kind = OutboundPushKind::from(request.delivery.resolution_request.delivery_kind());
    let payload_kind = outbound_push_kind_for_payload(&request.payload);

    let Some(decision) = outbound_policy
        .prepare_communication_delivery_attempt(request.delivery, communication_preferences)
        .await?
    else {
        return Ok(ProductOutboundDeliveryOutcome::NoDelivery);
    };

    let (attempt, target) = match decision {
        OutboundDeliveryDecision::Authorized { attempt, target } => (attempt, target),
        OutboundDeliveryDecision::Rejected { attempt } => {
            return Ok(ProductOutboundDeliveryOutcome::Rejected { attempt });
        }
    };

    if payload_kind != Some(delivery_kind) {
        let status_update_error =
            mark_attempt_failed(outbound_policy, &attempt, DeliveryFailureKind::Rejected).await;
        return Err(ProductOutboundDeliveryError::PayloadKindMismatch {
            delivery_kind,
            payload_kind,
            status_update_error,
        });
    }

    let metadata = match target_resolver
        .resolve_product_outbound_target_metadata(&target)
        .await
    {
        Ok(metadata) => metadata,
        Err(error) => {
            let status_update_error = mark_attempt_failed(
                outbound_policy,
                &attempt,
                delivery_failure_kind_for_workflow_error(&error),
            )
            .await;
            return Err(ProductOutboundDeliveryError::Workflow {
                source: error,
                status_update_error,
            });
        }
    };
    let product_target = ProductOutboundTarget::new(
        target.target().clone(),
        metadata.external_conversation_ref,
        metadata.external_actor_ref,
    );

    let envelope = ProductOutboundEnvelope {
        adapter_id: request.adapter.adapter_id().clone(),
        installation_id: request.adapter.installation_id().clone(),
        target: product_target,
        projection_cursor: request.projection_cursor,
        payload: request.payload,
        delivery_attempt_id: attempt.delivery_id.as_uuid(),
    };
    let render_result = request
        .adapter
        .render_outbound(envelope, request.egress, request.delivery_sink)
        .await;
    let render_outcome = match render_result {
        Ok(render_outcome) => render_outcome,
        Err(error) => {
            let status_update_error = mark_attempt_failed(
                outbound_policy,
                &attempt,
                delivery_failure_kind_for_adapter_error(&error),
            )
            .await;
            return Err(ProductOutboundDeliveryError::Adapter {
                source: error,
                status_update_error,
            });
        }
    };

    match render_outcome {
        ProductRenderOutcome::DeliveryRecorded | ProductRenderOutcome::SynchronousResponse(_) => {
            let status_update_error = outbound_policy
                .update_delivery_status(UpdateDeliveryStatusRequest {
                    delivery_id: attempt.delivery_id,
                    scope: attempt.scope.clone(),
                    status: OutboundDeliveryStatus::Delivered,
                    updated_at: chrono::Utc::now(),
                    failure_kind: None,
                })
                .await
                .err()
                .map(ProductOutboundStatusUpdateFailure::from);
            if let Some(status_update_error) = status_update_error {
                return Ok(ProductOutboundDeliveryOutcome::RenderedStatusUpdateFailed {
                    attempt,
                    render_outcome,
                    status_update_error,
                });
            }
        }
        ProductRenderOutcome::Deferred => {
            // Deferred means the adapter accepted the outbound attempt and will complete
            // it later through a product-specific completion or reconciliation path.
            // Leave the attempt Pending here so that later flow owns the final status.
        }
    }

    Ok(ProductOutboundDeliveryOutcome::Rendered {
        attempt,
        render_outcome,
    })
}

fn outbound_push_kind_for_payload(payload: &ProductOutboundPayload) -> Option<OutboundPushKind> {
    match payload {
        ProductOutboundPayload::FinalReply(_) => Some(OutboundPushKind::FinalReply),
        ProductOutboundPayload::Progress(_) => Some(OutboundPushKind::Progress),
        ProductOutboundPayload::GatePrompt(_) => Some(OutboundPushKind::GateRequired),
        ProductOutboundPayload::AuthPrompt(_) => Some(OutboundPushKind::AuthPrompt),
        ProductOutboundPayload::CapabilityActivity(_)
        | ProductOutboundPayload::CapabilityDisplayPreview(_)
        | ProductOutboundPayload::ProjectionSnapshot { .. }
        | ProductOutboundPayload::ProjectionUpdate { .. }
        | ProductOutboundPayload::KeepAlive => None,
    }
}

async fn mark_attempt_failed(
    outbound_policy: &OutboundPolicyService<'_>,
    attempt: &OutboundDeliveryAttempt,
    failure_kind: DeliveryFailureKind,
) -> Option<ProductOutboundStatusUpdateFailure> {
    outbound_policy
        .update_delivery_status(UpdateDeliveryStatusRequest {
            delivery_id: attempt.delivery_id,
            scope: attempt.scope.clone(),
            status: OutboundDeliveryStatus::Failed,
            updated_at: chrono::Utc::now(),
            failure_kind: Some(failure_kind),
        })
        .await
        .err()
        .map(|error| {
            let status_update_error = ProductOutboundStatusUpdateFailure::from(error);
            debug!(
                delivery_id = %attempt.delivery_id,
                failure_kind = ?failure_kind,
                status_update_error = ?status_update_error,
                "failed to mark outbound delivery as failed"
            );
            status_update_error
        })
}

fn delivery_failure_kind_for_adapter_error(error: &ProductAdapterError) -> DeliveryFailureKind {
    if error.is_retryable() {
        return DeliveryFailureKind::TransportUnavailable;
    }
    match error {
        ProductAdapterError::EgressDenied { .. }
        | ProductAdapterError::EgressUndeclaredHost { .. }
        | ProductAdapterError::InvalidIdentifier { .. }
        | ProductAdapterError::WorkflowRejected {
            retryable: false, ..
        } => DeliveryFailureKind::Rejected,
        _ => DeliveryFailureKind::Unknown,
    }
}

fn delivery_failure_kind_for_workflow_error(error: &ProductWorkflowError) -> DeliveryFailureKind {
    match error {
        ProductWorkflowError::Transient { .. } => DeliveryFailureKind::TransportUnavailable,
        ProductWorkflowError::BindingAccessDenied
        | ProductWorkflowError::BindingRequired { .. }
        | ProductWorkflowError::UnknownInstallation
        | ProductWorkflowError::InvalidBindingRequest { .. } => DeliveryFailureKind::Rejected,
        _ => DeliveryFailureKind::Unknown,
    }
}

impl From<OutboundError> for ProductOutboundStatusUpdateFailure {
    fn from(error: OutboundError) -> Self {
        match error {
            OutboundError::Backend => Self::Backend,
            OutboundError::Serialization => Self::Serialization,
            OutboundError::InvalidRequest { .. } => Self::InvalidRequest,
            OutboundError::SubscriptionScopeMismatch => Self::ScopeMismatch,
            OutboundError::AccessDenied => Self::AccessDenied,
            OutboundError::DeliveryNotFound => Self::DeliveryNotFound,
            OutboundError::CasConflict => Self::CasConflict,
            // PreferenceTargetMissing is a resolution-stage failure; it does not
            // reach the transport layer and is not a recognised status-update
            // category. Map to InvalidRequest so the saga does not retry.
            OutboundError::PreferenceTargetMissing { .. } => Self::InvalidRequest,
        }
    }
}
