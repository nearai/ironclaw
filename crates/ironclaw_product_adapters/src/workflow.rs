//! ProductWorkflow facade contract.
//!
//! Adapters call exactly one method on this facade — `accept_inbound` — to
//! drive the canonical pipeline. The workflow is responsible for:
//!
//! * resolving the binding (canonical user/thread) for the external refs;
//! * deduping by `(adapter_installation_id, source_binding, external_event_id)`;
//! * staging attachments into durable refs;
//! * submitting (or deferring) the turn through the kernel TurnCoordinator;
//! * returning a structured [`crate::ProductInboundAck`].
//!
//! Adapters MUST NOT call `ironclaw_turns::TurnCoordinator` themselves.

use async_trait::async_trait;

use crate::error::ProductAdapterError;
use crate::inbound::{ProductInboundAck, ProductInboundEnvelope};

#[async_trait]
pub trait ProductWorkflow: Send + Sync {
    async fn accept_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError>;
}
