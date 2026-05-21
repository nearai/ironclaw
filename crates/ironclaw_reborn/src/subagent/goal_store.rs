use std::collections::{HashMap, VecDeque};
#[cfg(any(feature = "filesystem-goal-store", test))]
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
#[cfg(feature = "filesystem-goal-store")]
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem,
};
#[cfg(feature = "filesystem-goal-store")]
use ironclaw_host_api::{ResourceScope, ScopedPath};
use ironclaw_turns::TurnRunId;
use serde::{Deserialize, Serialize};

pub const MAX_GOAL_ENTRIES: usize = 4096;
pub const MAX_GOAL_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentGoal {
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}

impl SubagentGoal {
    fn byte_len(&self) -> usize {
        self.task.len() + self.handoff.as_deref().map_or(0, str::len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SubagentGoalStoreError {
    #[error("subagent goal for run {run_id} not found")]
    NotFound { run_id: TurnRunId },
    #[error("subagent goal payload too large: {bytes} bytes (max {max})")]
    PayloadTooLarge { bytes: usize, max: usize },
    #[error("subagent goal for run {run_id} already stored")]
    DuplicateKey { run_id: TurnRunId },
    #[error("subagent goal store backend failed: {reason}")]
    Backend { reason: String },
}

#[async_trait]
pub trait SubagentGoalStore: Send + Sync {
    async fn put_goal(
        &self,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError>;

    async fn get_goal(&self, run_id: TurnRunId) -> Result<SubagentGoal, SubagentGoalStoreError>;

    async fn delete_goal(&self, run_id: TurnRunId) -> Result<(), SubagentGoalStoreError>;
}

#[cfg(feature = "filesystem-goal-store")]
fn validate_goal(goal: &SubagentGoal) -> Result<(), SubagentGoalStoreError> {
    let bytes = goal.byte_len();
    if bytes > MAX_GOAL_BYTES {
        return Err(SubagentGoalStoreError::PayloadTooLarge {
            bytes,
            max: MAX_GOAL_BYTES,
        });
    }
    Ok(())
}

#[cfg(feature = "filesystem-goal-store")]
pub struct FilesystemSubagentGoalStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

#[cfg(feature = "filesystem-goal-store")]
impl<F> FilesystemSubagentGoalStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }
}

/// Production subagent goal store over the same scoped filesystem substrate
/// used by Reborn turn state. In libSQL and PostgreSQL production composition,
/// the scoped filesystem is backed by the selected database root filesystem.
#[cfg(feature = "filesystem-goal-store")]
pub struct DbBackedSubagentGoalStore<F>
where
    F: RootFilesystem,
{
    inner: FilesystemSubagentGoalStore<F>,
}

#[cfg(feature = "filesystem-goal-store")]
impl<F> DbBackedSubagentGoalStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            inner: FilesystemSubagentGoalStore::new(filesystem),
        }
    }
}

#[cfg(feature = "filesystem-goal-store")]
#[async_trait]
impl<F> SubagentGoalStore for DbBackedSubagentGoalStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn put_goal(
        &self,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError> {
        self.inner.put_goal(run_id, goal).await
    }

    async fn get_goal(&self, run_id: TurnRunId) -> Result<SubagentGoal, SubagentGoalStoreError> {
        self.inner.get_goal(run_id).await
    }

    async fn delete_goal(&self, run_id: TurnRunId) -> Result<(), SubagentGoalStoreError> {
        self.inner.delete_goal(run_id).await
    }
}

#[cfg(feature = "filesystem-goal-store")]
#[async_trait]
impl<F> SubagentGoalStore for FilesystemSubagentGoalStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn put_goal(
        &self,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError> {
        validate_goal(&goal)?;
        let body = serde_json::to_vec(&goal).map_err(|error| SubagentGoalStoreError::Backend {
            reason: format!("subagent goal serialization failed: {error}"),
        })?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        match self
            .filesystem
            .put(
                &ResourceScope::system(),
                &goal_path(run_id)?,
                entry,
                CasExpectation::Absent,
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                Err(SubagentGoalStoreError::DuplicateKey { run_id })
            }
            Err(error) => Err(fs_backend_error(error)),
        }
    }

    async fn get_goal(&self, run_id: TurnRunId) -> Result<SubagentGoal, SubagentGoalStoreError> {
        let Some(versioned) = self
            .filesystem
            .get(&ResourceScope::system(), &goal_path(run_id)?)
            .await
            .map_err(fs_backend_error)?
        else {
            return Err(SubagentGoalStoreError::NotFound { run_id });
        };
        serde_json::from_slice(&versioned.entry.body).map_err(|error| {
            SubagentGoalStoreError::Backend {
                reason: format!("subagent goal deserialization failed: {error}"),
            }
        })
    }

    async fn delete_goal(&self, run_id: TurnRunId) -> Result<(), SubagentGoalStoreError> {
        match self
            .filesystem
            .delete(&ResourceScope::system(), &goal_path(run_id)?)
            .await
        {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(fs_backend_error(error)),
        }
    }
}

#[cfg(feature = "filesystem-goal-store")]
fn goal_path(run_id: TurnRunId) -> Result<ScopedPath, SubagentGoalStoreError> {
    ScopedPath::new(format!("/turns/subagent-goals/{}.json", run_id.as_uuid())).map_err(|error| {
        SubagentGoalStoreError::Backend {
            reason: format!("invalid subagent goal path: {error}"),
        }
    })
}

#[cfg(feature = "filesystem-goal-store")]
fn fs_backend_error(error: FilesystemError) -> SubagentGoalStoreError {
    SubagentGoalStoreError::Backend {
        reason: error.to_string(),
    }
}

#[derive(Default)]
pub struct BoundedSubagentGoalStore {
    inner: Mutex<GoalStoreInner>,
}

#[derive(Default)]
struct GoalStoreInner {
    goals: HashMap<TurnRunId, SubagentGoal>,
    insertion_order: VecDeque<TurnRunId>,
}

impl BoundedSubagentGoalStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, run_id: TurnRunId, goal: SubagentGoal) -> Result<(), SubagentGoalStoreError> {
        let bytes = goal.byte_len();
        if bytes > MAX_GOAL_BYTES {
            return Err(SubagentGoalStoreError::PayloadTooLarge {
                bytes,
                max: MAX_GOAL_BYTES,
            });
        }
        let mut inner = lock(&self.inner);
        if inner.goals.contains_key(&run_id) {
            return Err(SubagentGoalStoreError::DuplicateKey { run_id });
        }
        if inner.goals.len() >= MAX_GOAL_ENTRIES {
            while let Some(oldest) = inner.insertion_order.pop_front() {
                if inner.goals.remove(&oldest).is_some() {
                    tracing::debug!(
                        evicted_run_id = %oldest,
                        "subagent goal store at capacity; evicted oldest goal"
                    );
                    break;
                }
            }
        }
        inner.goals.insert(run_id, goal);
        inner.insertion_order.push_back(run_id);
        Ok(())
    }

    pub fn get(&self, run_id: TurnRunId) -> Result<SubagentGoal, SubagentGoalStoreError> {
        let inner = lock(&self.inner);
        inner
            .goals
            .get(&run_id)
            .cloned()
            .ok_or(SubagentGoalStoreError::NotFound { run_id })
    }

    fn delete(&self, run_id: TurnRunId) {
        let mut inner = lock(&self.inner);
        inner.goals.remove(&run_id);
        inner.insertion_order.retain(|queued| *queued != run_id);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        lock(&self.inner).goals.len()
    }

    #[cfg(test)]
    fn insertion_order_len(&self) -> usize {
        lock(&self.inner).insertion_order.len()
    }
}

#[async_trait]
impl SubagentGoalStore for BoundedSubagentGoalStore {
    async fn put_goal(
        &self,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError> {
        self.put(run_id, goal)
    }

    async fn get_goal(&self, run_id: TurnRunId) -> Result<SubagentGoal, SubagentGoalStoreError> {
        self.get(run_id)
    }

    async fn delete_goal(&self, run_id: TurnRunId) -> Result<(), SubagentGoalStoreError> {
        self.delete(run_id);
        Ok(())
    }
}

fn lock(inner: &Mutex<GoalStoreInner>) -> MutexGuard<'_, GoalStoreInner> {
    match inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "filesystem-goal-store")]
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    #[cfg(feature = "filesystem-goal-store")]
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    fn goal(task: &str) -> SubagentGoal {
        SubagentGoal {
            task: task.to_string(),
            handoff: None,
        }
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let store = BoundedSubagentGoalStore::new();
        let run_id = TurnRunId::new();
        let expected = SubagentGoal {
            task: "summarize this".to_string(),
            handoff: Some("context".to_string()),
        };

        store.put_goal(run_id, expected.clone()).await.unwrap();

        assert_eq!(store.get_goal(run_id).await.unwrap(), expected);
    }

    #[tokio::test]
    async fn get_miss_is_not_found_error() {
        let store = BoundedSubagentGoalStore::new();
        let run_id = TurnRunId::new();

        assert_eq!(
            store.get_goal(run_id).await.unwrap_err(),
            SubagentGoalStoreError::NotFound { run_id }
        );
    }

    #[tokio::test]
    async fn put_rejects_oversized_payload() {
        let store = BoundedSubagentGoalStore::new();
        let run_id = TurnRunId::new();
        let large = SubagentGoal {
            task: "x".repeat(MAX_GOAL_BYTES + 1),
            handoff: None,
        };

        assert!(matches!(
            store.put_goal(run_id, large).await,
            Err(SubagentGoalStoreError::PayloadTooLarge { .. })
        ));
    }

    #[tokio::test]
    async fn put_rejects_duplicate_key() {
        let store = BoundedSubagentGoalStore::new();
        let run_id = TurnRunId::new();

        store.put_goal(run_id, goal("first")).await.unwrap();

        assert_eq!(
            store.put_goal(run_id, goal("second")).await.unwrap_err(),
            SubagentGoalStoreError::DuplicateKey { run_id }
        );
    }

    #[tokio::test]
    async fn bounded_store_evicts_oldest() {
        let store = BoundedSubagentGoalStore::new();
        let first = TurnRunId::new();
        let second = TurnRunId::new();
        store.put_goal(first, goal("first")).await.unwrap();
        store.put_goal(second, goal("second")).await.unwrap();
        for index in 2..=MAX_GOAL_ENTRIES {
            store
                .put_goal(TurnRunId::new(), goal(&format!("goal-{index}")))
                .await
                .unwrap();
        }

        assert!(matches!(
            store.get_goal(first).await,
            Err(SubagentGoalStoreError::NotFound { .. })
        ));
        assert_eq!(store.get_goal(second).await.unwrap(), goal("second"));
        assert_eq!(store.len(), MAX_GOAL_ENTRIES);
    }

    #[tokio::test]
    async fn delete_goal_is_idempotent_and_removes_row() {
        let store = BoundedSubagentGoalStore::new();
        let run_id = TurnRunId::new();

        store.put_goal(run_id, goal("task")).await.unwrap();
        store.delete_goal(run_id).await.unwrap();
        store.delete_goal(run_id).await.unwrap();

        assert!(matches!(
            store.get_goal(run_id).await,
            Err(SubagentGoalStoreError::NotFound { .. })
        ));
        assert_eq!(store.insertion_order_len(), 0);
    }

    #[cfg(feature = "filesystem-goal-store")]
    fn scoped_goal_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::new()),
            mounts,
        ))
    }

    #[cfg(feature = "filesystem-goal-store")]
    async fn assert_goal_store_contract(store: &dyn SubagentGoalStore) {
        let run_id = TurnRunId::new();
        let expected = SubagentGoal {
            task: "durable task".to_string(),
            handoff: Some("handoff".to_string()),
        };

        store.put_goal(run_id, expected.clone()).await.unwrap();
        assert_eq!(store.get_goal(run_id).await.unwrap(), expected);
        assert_eq!(
            store.put_goal(run_id, goal("duplicate")).await.unwrap_err(),
            SubagentGoalStoreError::DuplicateKey { run_id }
        );
        assert!(matches!(
            store
                .put_goal(
                    TurnRunId::new(),
                    SubagentGoal {
                        task: "x".repeat(MAX_GOAL_BYTES + 1),
                        handoff: None,
                    },
                )
                .await,
            Err(SubagentGoalStoreError::PayloadTooLarge { .. })
        ));
        store.delete_goal(run_id).await.unwrap();
        store.delete_goal(run_id).await.unwrap();
        assert!(matches!(
            store.get_goal(run_id).await,
            Err(SubagentGoalStoreError::NotFound { .. })
        ));
    }

    #[cfg(feature = "filesystem-goal-store")]
    #[tokio::test]
    async fn db_backed_goal_store_satisfies_subagent_goal_contract() {
        let store = DbBackedSubagentGoalStore::new(scoped_goal_filesystem());
        assert_goal_store_contract(&store).await;
    }

    #[cfg(feature = "filesystem-goal-store")]
    #[tokio::test]
    async fn db_backed_goal_store_reopens_over_same_backend() {
        let filesystem = scoped_goal_filesystem();
        let first = DbBackedSubagentGoalStore::new(Arc::clone(&filesystem));
        let run_id = TurnRunId::new();
        let expected = goal("survives reopen");

        first.put_goal(run_id, expected.clone()).await.unwrap();
        let reopened = DbBackedSubagentGoalStore::new(filesystem);

        assert_eq!(reopened.get_goal(run_id).await.unwrap(), expected);
    }

    #[test]
    fn goal_store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<dyn SubagentGoalStore>>();
        assert_send_sync::<BoundedSubagentGoalStore>();
        #[cfg(feature = "filesystem-goal-store")]
        assert_send_sync::<DbBackedSubagentGoalStore<InMemoryBackend>>();
        #[cfg(feature = "filesystem-goal-store")]
        assert_send_sync::<FilesystemSubagentGoalStore<InMemoryBackend>>();
    }
}
