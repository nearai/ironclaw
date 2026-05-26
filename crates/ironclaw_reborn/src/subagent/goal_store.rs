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
use ironclaw_host_api::ScopedPath;
use ironclaw_turns::{TurnRunId, TurnScope};
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
        serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len())
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
        scope: &TurnScope,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError>;

    async fn get_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<SubagentGoal, SubagentGoalStoreError>;

    async fn delete_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<(), SubagentGoalStoreError>;
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

// `FilesystemSubagentGoalStore` is the production subagent goal store. In
// libSQL and PostgreSQL production composition, the scoped filesystem passed
// in is backed by the selected database root filesystem — that distinction
// belongs in the choice of `F` at the call site, not in a separate wrapper
// type.

#[cfg(feature = "filesystem-goal-store")]
#[async_trait]
impl<F> SubagentGoalStore for FilesystemSubagentGoalStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn put_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError> {
        validate_goal(&goal)?;
        let body = serde_json::to_vec(&goal).map_err(|error| SubagentGoalStoreError::Backend {
            reason: format!("subagent goal serialization failed: {error}"),
        })?;
        let entry = Entry::bytes(body).with_content_type(ContentType::json());
        let resource_scope = scope.to_resource_scope();
        match self
            .filesystem
            .put(
                &resource_scope,
                &goal_path(scope, run_id)?,
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

    async fn get_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<SubagentGoal, SubagentGoalStoreError> {
        let resource_scope = scope.to_resource_scope();
        let Some(versioned) = self
            .filesystem
            .get(&resource_scope, &goal_path(scope, run_id)?)
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

    async fn delete_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<(), SubagentGoalStoreError> {
        let resource_scope = scope.to_resource_scope();
        match self
            .filesystem
            .delete(&resource_scope, &goal_path(scope, run_id)?)
            .await
        {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(fs_backend_error(error)),
        }
    }
}

#[cfg(feature = "filesystem-goal-store")]
fn goal_path(scope: &TurnScope, run_id: TurnRunId) -> Result<ScopedPath, SubagentGoalStoreError> {
    let mut path = String::from("/turns/subagent-goals");
    if let Some(agent_id) = &scope.agent_id {
        path.push_str("/agents/");
        path.push_str(agent_id.as_str());
    }
    if let Some(project_id) = &scope.project_id {
        path.push_str("/projects/");
        path.push_str(project_id.as_str());
    }
    path.push_str("/threads/");
    path.push_str(scope.thread_id.as_str());
    path.push('/');
    path.push_str(&run_id.as_uuid().to_string());
    path.push_str(".json");
    ScopedPath::new(path).map_err(|error| SubagentGoalStoreError::Backend {
        reason: format!("invalid subagent goal path: {error}"),
    })
}

#[cfg(feature = "filesystem-goal-store")]
fn fs_backend_error(error: FilesystemError) -> SubagentGoalStoreError {
    SubagentGoalStoreError::Backend {
        reason: error.to_string(),
    }
}

#[derive(Default)]
pub struct InMemoryBoundedSubagentGoalStore {
    inner: Mutex<GoalStoreInner>,
}

#[derive(Default)]
struct GoalStoreInner {
    goals: HashMap<GoalKey, SubagentGoal>,
    insertion_order: VecDeque<GoalKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GoalKey {
    scope: TurnScope,
    run_id: TurnRunId,
}

impl GoalKey {
    fn new(scope: &TurnScope, run_id: TurnRunId) -> Self {
        Self {
            scope: scope.clone(),
            run_id,
        }
    }
}

impl InMemoryBoundedSubagentGoalStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError> {
        let bytes = goal.byte_len();
        if bytes > MAX_GOAL_BYTES {
            return Err(SubagentGoalStoreError::PayloadTooLarge {
                bytes,
                max: MAX_GOAL_BYTES,
            });
        }
        let mut inner = lock(&self.inner);
        let key = GoalKey::new(scope, run_id);
        if inner.goals.contains_key(&key) {
            return Err(SubagentGoalStoreError::DuplicateKey { run_id });
        }
        if inner.goals.len() >= MAX_GOAL_ENTRIES {
            while let Some(oldest) = inner.insertion_order.pop_front() {
                if inner.goals.remove(&oldest).is_some() {
                    tracing::debug!(
                        evicted_run_id = %oldest.run_id,
                        "subagent goal store at capacity; evicted oldest goal"
                    );
                    break;
                }
            }
        }
        inner.goals.insert(key.clone(), goal);
        inner.insertion_order.push_back(key);
        Ok(())
    }

    pub fn get(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<SubagentGoal, SubagentGoalStoreError> {
        let inner = lock(&self.inner);
        inner
            .goals
            .get(&GoalKey::new(scope, run_id))
            .cloned()
            .ok_or(SubagentGoalStoreError::NotFound { run_id })
    }

    fn delete_inner(&self, scope: &TurnScope, run_id: TurnRunId) {
        let mut inner = lock(&self.inner);
        let key = GoalKey::new(scope, run_id);
        inner.goals.remove(&key);
        inner.insertion_order.retain(|queued| *queued != key);
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
impl SubagentGoalStore for InMemoryBoundedSubagentGoalStore {
    async fn put_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        goal: SubagentGoal,
    ) -> Result<(), SubagentGoalStoreError> {
        self.put(scope, run_id, goal)
    }

    async fn get_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<SubagentGoal, SubagentGoalStoreError> {
        self.get(scope, run_id)
    }

    async fn delete_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<(), SubagentGoalStoreError> {
        self.delete_inner(scope, run_id);
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
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    #[cfg(feature = "filesystem-goal-store")]
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    fn scope(thread_id: &str) -> TurnScope {
        TurnScope::new(
            TenantId::new("tenant-alpha").unwrap(),
            Some(AgentId::new("agent-alpha").unwrap()),
            Some(ProjectId::new("project-alpha").unwrap()),
            ThreadId::new(thread_id).unwrap(),
        )
    }

    fn goal(task: &str) -> SubagentGoal {
        SubagentGoal {
            task: task.to_string(),
            handoff: None,
        }
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let store = InMemoryBoundedSubagentGoalStore::new();
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();
        let expected = SubagentGoal {
            task: "summarize this".to_string(),
            handoff: Some("context".to_string()),
        };

        store
            .put_goal(&owner_scope, run_id, expected.clone())
            .await
            .unwrap();

        assert_eq!(
            store.get_goal(&owner_scope, run_id).await.unwrap(),
            expected
        );
    }

    #[tokio::test]
    async fn get_miss_is_not_found_error() {
        let store = InMemoryBoundedSubagentGoalStore::new();
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();

        assert_eq!(
            store.get_goal(&owner_scope, run_id).await.unwrap_err(),
            SubagentGoalStoreError::NotFound { run_id }
        );
    }

    #[tokio::test]
    async fn put_rejects_oversized_payload() {
        let store = InMemoryBoundedSubagentGoalStore::new();
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();
        let large = SubagentGoal {
            task: "x".repeat(MAX_GOAL_BYTES + 1),
            handoff: None,
        };

        assert!(matches!(
            store.put_goal(&owner_scope, run_id, large).await,
            Err(SubagentGoalStoreError::PayloadTooLarge { .. })
        ));
    }

    #[tokio::test]
    async fn put_rejects_payload_when_json_overhead_exceeds_limit() {
        let store = InMemoryBoundedSubagentGoalStore::new();
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();
        let large = SubagentGoal {
            task: "x".repeat(MAX_GOAL_BYTES - 8),
            handoff: None,
        };

        assert!(
            large.task.len() <= MAX_GOAL_BYTES,
            "raw string payload stays below the limit"
        );
        assert!(matches!(
            store.put_goal(&owner_scope, run_id, large).await,
            Err(SubagentGoalStoreError::PayloadTooLarge { .. })
        ));
    }

    #[tokio::test]
    async fn put_rejects_duplicate_key() {
        let store = InMemoryBoundedSubagentGoalStore::new();
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();

        store
            .put_goal(&owner_scope, run_id, goal("first"))
            .await
            .unwrap();

        assert_eq!(
            store
                .put_goal(&owner_scope, run_id, goal("second"))
                .await
                .unwrap_err(),
            SubagentGoalStoreError::DuplicateKey { run_id }
        );
    }

    #[tokio::test]
    async fn bounded_store_evicts_oldest() {
        let store = InMemoryBoundedSubagentGoalStore::new();
        let owner_scope = scope("thread-goal");
        let first = TurnRunId::new();
        let second = TurnRunId::new();
        store
            .put_goal(&owner_scope, first, goal("first"))
            .await
            .unwrap();
        store
            .put_goal(&owner_scope, second, goal("second"))
            .await
            .unwrap();
        for index in 2..=MAX_GOAL_ENTRIES {
            store
                .put_goal(
                    &owner_scope,
                    TurnRunId::new(),
                    goal(&format!("goal-{index}")),
                )
                .await
                .unwrap();
        }

        assert!(matches!(
            store.get_goal(&owner_scope, first).await,
            Err(SubagentGoalStoreError::NotFound { .. })
        ));
        assert_eq!(
            store.get_goal(&owner_scope, second).await.unwrap(),
            goal("second")
        );
        assert_eq!(store.len(), MAX_GOAL_ENTRIES);
    }

    #[tokio::test]
    async fn delete_goal_is_idempotent_and_removes_row() {
        let store = InMemoryBoundedSubagentGoalStore::new();
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();

        store
            .put_goal(&owner_scope, run_id, goal("task"))
            .await
            .unwrap();
        store.delete_goal(&owner_scope, run_id).await.unwrap();
        store.delete_goal(&owner_scope, run_id).await.unwrap();

        assert!(matches!(
            store.get_goal(&owner_scope, run_id).await,
            Err(SubagentGoalStoreError::NotFound { .. })
        ));
        assert_eq!(store.insertion_order_len(), 0);
    }

    #[tokio::test]
    async fn bounded_store_keys_goals_by_scope_and_run_id() {
        let store = InMemoryBoundedSubagentGoalStore::new();
        let first_scope = scope("thread-goal-a");
        let second_scope = scope("thread-goal-b");
        let run_id = TurnRunId::new();

        store
            .put_goal(&first_scope, run_id, goal("scoped task"))
            .await
            .unwrap();
        assert!(matches!(
            store.get_goal(&second_scope, run_id).await,
            Err(SubagentGoalStoreError::NotFound { .. })
        ));

        store.delete_goal(&second_scope, run_id).await.unwrap();

        assert_eq!(
            store.get_goal(&first_scope, run_id).await.unwrap(),
            goal("scoped task")
        );
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
        let owner_scope = scope("thread-goal");
        let other_scope = scope("thread-goal-other");
        let run_id = TurnRunId::new();
        let expected = SubagentGoal {
            task: "durable task".to_string(),
            handoff: Some("handoff".to_string()),
        };

        store
            .put_goal(&owner_scope, run_id, expected.clone())
            .await
            .unwrap();
        assert_eq!(
            store.get_goal(&owner_scope, run_id).await.unwrap(),
            expected
        );
        assert!(matches!(
            store.get_goal(&other_scope, run_id).await,
            Err(SubagentGoalStoreError::NotFound { .. })
        ));
        store.delete_goal(&other_scope, run_id).await.unwrap();
        assert!(store.get_goal(&owner_scope, run_id).await.is_ok());
        assert_eq!(
            store
                .put_goal(&owner_scope, run_id, goal("duplicate"))
                .await
                .unwrap_err(),
            SubagentGoalStoreError::DuplicateKey { run_id }
        );
        assert!(matches!(
            store
                .put_goal(
                    &owner_scope,
                    TurnRunId::new(),
                    SubagentGoal {
                        task: "x".repeat(MAX_GOAL_BYTES + 1),
                        handoff: None,
                    },
                )
                .await,
            Err(SubagentGoalStoreError::PayloadTooLarge { .. })
        ));
        store.delete_goal(&owner_scope, run_id).await.unwrap();
        store.delete_goal(&owner_scope, run_id).await.unwrap();
        assert!(matches!(
            store.get_goal(&owner_scope, run_id).await,
            Err(SubagentGoalStoreError::NotFound { .. })
        ));
    }

    #[cfg(feature = "filesystem-goal-store")]
    #[tokio::test]
    async fn filesystem_goal_store_satisfies_subagent_goal_contract() {
        let store = FilesystemSubagentGoalStore::new(scoped_goal_filesystem());
        assert_goal_store_contract(&store).await;
    }

    #[cfg(feature = "filesystem-goal-store")]
    #[test]
    fn filesystem_goal_path_uses_alias_relative_named_scope_axes() {
        let owner_scope = scope("thread-goal-path");
        let run_id = TurnRunId::new();

        let path = goal_path(&owner_scope, run_id).unwrap();

        assert_eq!(
            path.as_str(),
            format!(
                "/turns/subagent-goals/agents/agent-alpha/projects/project-alpha/threads/thread-goal-path/{}.json",
                run_id.as_uuid()
            )
        );
        assert!(
            !path.as_str().contains("tenant-alpha"),
            "resource scope already supplies tenant isolation"
        );
    }

    #[cfg(feature = "filesystem-goal-store")]
    #[tokio::test]
    async fn filesystem_goal_store_reopens_over_same_backend() {
        let filesystem = scoped_goal_filesystem();
        let first = FilesystemSubagentGoalStore::new(Arc::clone(&filesystem));
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();
        let expected = goal("survives reopen");

        first
            .put_goal(&owner_scope, run_id, expected.clone())
            .await
            .unwrap();
        let reopened = FilesystemSubagentGoalStore::new(filesystem);

        assert_eq!(
            reopened.get_goal(&owner_scope, run_id).await.unwrap(),
            expected
        );
    }

    #[test]
    fn goal_store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<dyn SubagentGoalStore>>();
        assert_send_sync::<InMemoryBoundedSubagentGoalStore>();
        #[cfg(feature = "filesystem-goal-store")]
        assert_send_sync::<FilesystemSubagentGoalStore<InMemoryBackend>>();
    }
}
