//! ProductWorkflow facade contract.

use async_trait::async_trait;

use crate::error::ProductAdapterError;
use crate::inbound::{ProductInboundAck, ProductInboundEnvelope};
use crate::projection::ProjectionSubscriptionRequest;

#[async_trait]
pub trait ProductWorkflow: Send + Sync {
    async fn accept_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError>;

    /// Resolve an adapter-level projection subscription request into the
    /// canonical actor/scope/cursor used by [`crate::ProjectionStream`].
    async fn resolve_projection_subscription(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError>;
}
