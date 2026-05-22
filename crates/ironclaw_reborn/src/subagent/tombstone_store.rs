use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use ironclaw_turns::TurnRunId;

use crate::subagent::spawn_result::SubagentResultTombstone;

const MAX_TOMBSTONE_RECORDS: usize = 4096;

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
    inner: std::sync::Mutex<TombstoneInner>,
}

#[derive(Default)]
struct TombstoneInner {
    by_child: HashMap<TurnRunId, SubagentResultTombstone>,
    insertion_order: VecDeque<TurnRunId>,
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
        let child_run_id = tombstone.child_run_id;
        if !inner.by_child.contains_key(&child_run_id) {
            inner.insertion_order.push_back(child_run_id);
        }
        inner.by_child.insert(child_run_id, tombstone);
        while inner.by_child.len() > MAX_TOMBSTONE_RECORDS {
            let Some(oldest) = inner.insertion_order.pop_front() else {
                break;
            };
            inner.by_child.remove(&oldest);
        }
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
        Ok(inner.by_child.get(&child_run_id).cloned())
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

    #[tokio::test]
    async fn tombstone_store_evicts_oldest_record_at_capacity() {
        let store = BoundedSubagentResultTombstoneStore::new();
        let first_child = TurnRunId::new();
        store.write_tombstone(tombstone(first_child)).await.unwrap();

        for _ in 1..=MAX_TOMBSTONE_RECORDS {
            store
                .write_tombstone(tombstone(TurnRunId::new()))
                .await
                .unwrap();
        }

        assert_eq!(store.read_tombstone(first_child).await.unwrap(), None);
    }

    fn tombstone(child_run_id: TurnRunId) -> SubagentResultTombstone {
        SubagentResultTombstone {
            child_run_id,
            terminal_status: TurnStatus::Cancelled,
            disposition: SubagentResultDisposition::DiscardedByParentCancel,
        }
    }
}
