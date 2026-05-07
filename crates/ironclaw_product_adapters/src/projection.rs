//! Projection subscription contract.
//!
//! Adapters consume projection-derived outputs through this trait. Production
//! implementations live in the projection-stream service (#3266 / #3093);
//! this crate ships only the contract and an in-memory fake.

use async_trait::async_trait;
use ironclaw_turns::{TurnActor, TurnScope};
use serde::{Deserialize, Serialize};

use crate::error::ProductAdapterError;
use crate::outbound::{ProductOutboundEnvelope, ProjectionCursor};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSubscriptionRequest {
    pub actor: TurnActor,
    pub scope: TurnScope,
    pub after_cursor: Option<ProjectionCursor>,
}

#[async_trait]
pub trait ProjectionStream: Send + Sync {
    /// Drain pending projection updates for the given subscription request.
    /// Production implementations stream; the contract returns a snapshot for
    /// fake-driven tests.
    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError>;
}
