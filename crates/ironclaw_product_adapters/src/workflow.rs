//! ProductWorkflow facade contract.
//!
//! Product surfaces must choose the ProductWorkflow door that matches their
//! effect boundary:
//!
//! | Product/API behavior | ProductWorkflow door |
//! | --- | --- |
//! | Create chat completion / response | [`ProductWorkflow::submit_inbound`] |
//! | Retrieve current response/projection | [`ProductWorkflow::read_projection`] |
//! | Stream response/projection events | [`ProductWorkflow::subscribe_projection`] |
//! | Cancel response/run | [`ProductWorkflow::submit_inbound`] with a typed control payload |
//!
//! The OpenAI-compatible layer may resolve opaque `resp_*` identifiers into
//! canonical Reborn projection metadata before calling the projection doors, but
//! route handlers should not fabricate inbound envelopes or bypass this facade to
//! call projection internals directly.

use async_trait::async_trait;

use crate::error::ProductAdapterError;
use crate::inbound::{ProductInboundAck, ProductInboundEnvelope};
use crate::projection::{
    ProductProjectionReadInput, ProductProjectionSubscribeInput, ProjectionReadRequest,
    ProjectionSubscriptionRequest,
};
use crate::redaction::RedactedString;

#[async_trait]
pub trait ProductWorkflow: Send + Sync {
    /// Submit a mutating product action into the ProductWorkflow submit/control
    /// path.
    ///
    /// This door is for payloads that can create messages, runs,
    /// command/gate/auth outcomes, mission work, typed control actions, or other
    /// durable side effects. Projection read/subscribe requests must use the
    /// projection doors and must not create mutating ProductInboundAction ledger
    /// rows.
    async fn submit_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError>;

    /// Resolve a ProductWorkflow-facing projection read/fetch request into the
    /// canonical actor/scope/cursor/window used by projection services.
    ///
    /// This door accepts typed projection input rather than requiring callers to
    /// fabricate a full inbound envelope. API routes may call it after resolving
    /// opaque product ids into canonical Reborn projection metadata.
    async fn read_projection(
        &self,
        _request: ProductProjectionReadInput,
    ) -> Result<ProjectionReadRequest, ProductAdapterError> {
        Err(ProductAdapterError::Internal {
            detail: RedactedString::new(
                "projection read is not supported by this ProductWorkflow implementation",
            ),
        })
    }

    /// Resolve a ProductWorkflow-facing projection subscription request into the
    /// canonical actor/scope/cursor used by [`crate::ProjectionStream`].
    ///
    /// This door accepts typed projection input rather than requiring callers to
    /// fabricate a full inbound envelope. API routes may call it after resolving
    /// opaque product ids into canonical Reborn projection metadata.
    async fn subscribe_projection(
        &self,
        _request: ProductProjectionSubscribeInput,
    ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
        Err(ProductAdapterError::Internal {
            detail: RedactedString::new(
                "projection subscription is not supported by this ProductWorkflow implementation",
            ),
        })
    }

    /// Compatibility wrapper for existing callers. New adapter/API wiring should
    /// call [`Self::submit_inbound`] explicitly.
    async fn accept_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError> {
        self.submit_inbound(envelope).await
    }

    /// Compatibility wrapper for legacy adapter-level subscription callers. New
    /// code should pass typed projection input to [`Self::subscribe_projection`].
    async fn resolve_projection_subscription(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
        self.subscribe_projection(ProductProjectionSubscribeInput::from_inbound_envelope(
            &envelope,
        )?)
        .await
    }
}
