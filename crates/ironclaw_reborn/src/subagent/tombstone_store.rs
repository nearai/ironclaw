use std::collections::HashMap;

use async_trait::async_trait;
use ironclaw_turns::TurnRunId;

use crate::subagent::spawn_result::SubagentResultTombstone;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TombstoneStoreError {
    #[error("subagent tombstone store backend failed: {reason}")]
    Backend { reason: String },
}

#[async_trait]
pub trait SubagentResultTombstoneStore: Send + Sync {
    async fn write_tombstone(
        &self,
        tombstone: SubagentResultTombstone,
    ) -> Result<(), TombstoneStoreError>;

    async fn read_tombstone(
        &self,
        child_run_id: TurnRunId,
    ) -> Result<Option<SubagentResultTombstone>, TombstoneStoreError>;
}

#[derive(Default)]
pub struct BoundedSubagentResultTombstoneStore {
    inner: std::sync::Mutex<HashMap<TurnRunId, SubagentResultTombstone>>,
}

impl BoundedSubagentResultTombstoneStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SubagentResultTombstoneStore for BoundedSubagentResultTombstoneStore {
    async fn write_tombstone(
        &self,
        tombstone: SubagentResultTombstone,
    ) -> Result<(), TombstoneStoreError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TombstoneStoreError::Backend {
                reason: "subagent tombstone store mutex poisoned".to_string(),
            })?;
        inner.insert(tombstone.child_run_id, tombstone);
        Ok(())
    }

    async fn read_tombstone(
        &self,
        child_run_id: TurnRunId,
    ) -> Result<Option<SubagentResultTombstone>, TombstoneStoreError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| TombstoneStoreError::Backend {
                reason: "subagent tombstone store mutex poisoned".to_string(),
            })?;
        Ok(inner.get(&child_run_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_turns::TurnStatus;

    use crate::subagent::spawn_result::{SubagentResultDisposition, SubagentResultTombstone};

    use super::*;

    #[tokio::test]
    async fn tombstone_store_is_idempotent_by_child_run() {
        let store = BoundedSubagentResultTombstoneStore::new();
        let child_run_id = TurnRunId::new();
        let tombstone = SubagentResultTombstone {
            child_run_id,
            terminal_status: TurnStatus::Cancelled,
            disposition: SubagentResultDisposition::DiscardedByParentCancel,
        };

        store.write_tombstone(tombstone.clone()).await.unwrap();
        store.write_tombstone(tombstone.clone()).await.unwrap();

        assert_eq!(
            store.read_tombstone(child_run_id).await.unwrap(),
            Some(tombstone)
        );
    }
}
