//! ProductWorkflow facade contract.

use async_trait::async_trait;

use crate::error::ProductAdapterError;
use crate::inbound::{ProductInboundAck, ProductInboundEnvelope};
use crate::projection::ProjectionSubscriptionRequest;

#[async_trait]
pub trait ProductWorkflow: Send + Sync {
    /// Accept a mutating product action into the ProductWorkflow submit path.
    ///
    /// This entrypoint is for payloads that can create messages, runs,
    /// command/gate/auth outcomes, or other durable side effects. Projection
    /// read/subscribe requests must use [`Self::resolve_projection_subscription`]
    /// and must not create mutating ProductInboundAction ledger rows.
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
