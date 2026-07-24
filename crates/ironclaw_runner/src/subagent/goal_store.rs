use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::ScopedPath;
use ironclaw_turns::{TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

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
pub trait SubagentGoalStorePort: Send + Sync {
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

pub struct SubagentGoalStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> SubagentGoalStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }
}

// `SubagentGoalStore` is the production subagent goal store. In
// libSQL and PostgreSQL production composition, the scoped filesystem passed
// in is backed by the selected database root filesystem — that distinction
// belongs in the choice of `F` at the call site, not in a separate wrapper
// type.

#[async_trait]
impl<F> SubagentGoalStorePort for SubagentGoalStore<F>
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

#[async_trait]
impl<F> ironclaw_loop_host::SubagentSpawnGoalStore for SubagentGoalStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn put_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        goal: ironclaw_loop_host::SubagentGoalRecord,
    ) -> Result<(), ironclaw_turns::run_profile::AgentLoopHostError> {
        <Self as SubagentGoalStorePort>::put_goal(
            self,
            scope,
            run_id,
            SubagentGoal {
                task: goal.task,
                handoff: goal.handoff,
            },
        )
        .await
        .map_err(map_goal_error)
    }

    async fn delete_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<(), ironclaw_turns::run_profile::AgentLoopHostError> {
        <Self as SubagentGoalStorePort>::delete_goal(self, scope, run_id)
            .await
            .map_err(map_goal_error)
    }
}

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

fn fs_backend_error(error: FilesystemError) -> SubagentGoalStoreError {
    SubagentGoalStoreError::Backend {
        reason: error.to_string(),
    }
}

fn map_goal_error(
    error: SubagentGoalStoreError,
) -> ironclaw_turns::run_profile::AgentLoopHostError {
    let kind = match error {
        SubagentGoalStoreError::NotFound { .. } => {
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::InvalidInvocation
        }
        SubagentGoalStoreError::PayloadTooLarge { .. } => {
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::BudgetExceeded
        }
        SubagentGoalStoreError::DuplicateKey { .. } => {
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::InvalidInvocation
        }
        SubagentGoalStoreError::Backend { .. } => {
            ironclaw_turns::run_profile::AgentLoopHostErrorKind::Unavailable
        }
    };
    ironclaw_turns::run_profile::AgentLoopHostError::new(kind, error.to_string())
}

#[cfg(any(test, feature = "test-support"))]
pub fn in_memory_backed_subagent_goal_store()
-> SubagentGoalStore<ironclaw_filesystem::InMemoryBackend> {
    SubagentGoalStore::new(scoped_goal_filesystem())
}

#[cfg(any(test, feature = "test-support"))]
pub fn scoped_goal_filesystem() -> Arc<ScopedFilesystem<ironclaw_filesystem::InMemoryBackend>> {
    use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("mount alias"),
        VirtualPath::new("/turns").expect("mount path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};

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
        let store = in_memory_backed_subagent_goal_store();
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
        let store = in_memory_backed_subagent_goal_store();
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();

        assert_eq!(
            store.get_goal(&owner_scope, run_id).await.unwrap_err(),
            SubagentGoalStoreError::NotFound { run_id }
        );
    }

    #[tokio::test]
    async fn put_rejects_oversized_payload() {
        let store = in_memory_backed_subagent_goal_store();
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
        let store = in_memory_backed_subagent_goal_store();
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
        let store = in_memory_backed_subagent_goal_store();
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
    async fn delete_goal_is_idempotent_and_removes_row() {
        let store = in_memory_backed_subagent_goal_store();
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
    }

    #[tokio::test]
    async fn goal_store_keys_goals_by_scope_and_run_id() {
        let store = in_memory_backed_subagent_goal_store();
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

    async fn assert_goal_store_contract(store: &dyn SubagentGoalStorePort) {
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

    #[tokio::test]
    async fn filesystem_goal_store_satisfies_subagent_goal_contract() {
        let store = SubagentGoalStore::new(scoped_goal_filesystem());
        assert_goal_store_contract(&store).await;
    }

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

    #[tokio::test]
    async fn filesystem_goal_store_reopens_over_same_backend() {
        let filesystem = scoped_goal_filesystem();
        let first = SubagentGoalStore::new(Arc::clone(&filesystem));
        let owner_scope = scope("thread-goal");
        let run_id = TurnRunId::new();
        let expected = goal("survives reopen");

        first
            .put_goal(&owner_scope, run_id, expected.clone())
            .await
            .unwrap();
        let reopened = SubagentGoalStore::new(filesystem);

        assert_eq!(
            reopened.get_goal(&owner_scope, run_id).await.unwrap(),
            expected
        );
    }

    #[test]
    fn goal_store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<dyn SubagentGoalStorePort>>();
        assert_send_sync::<SubagentGoalStore<InMemoryBackend>>();
    }
}
