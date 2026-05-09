//! Projection subscription contract.

use async_trait::async_trait;
use ironclaw_turns::{TurnActor, TurnScope};
use serde::{Deserialize, Serialize};

use crate::error::ProductAdapterError;
use crate::outbound::{ProductOutboundEnvelope, ProjectionCursor};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectionSubscriptionRequest {
    pub actor: TurnActor,
    pub scope: TurnScope,
    pub after_cursor: Option<ProjectionCursor>,
}

#[async_trait]
pub trait ProjectionStream: Send + Sync {
    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError>;
}
