use async_trait::async_trait;

use crate::{InboundTurnError, memory::InMemoryState};

pub(crate) struct PersistedConversationState {
    pub(crate) state: InMemoryState,
    pub(crate) revision: i64,
}

#[async_trait]
pub(crate) trait ConversationStateRepository: Send + Sync {
    async fn load_state(&self) -> Result<PersistedConversationState, InboundTurnError>;
    async fn save_state(
        &self,
        expected_revision: i64,
        state: &InMemoryState,
    ) -> Result<i64, InboundTurnError>;
}
