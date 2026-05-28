use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{
    SubagentPromptGoal, SubagentPromptMaterial, SubagentPromptMaterialSource, SubagentThreadKind,
    SubagentThreadMetadata,
};
use ironclaw_threads::{
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::run_profile::{AgentLoopHostError, AgentLoopHostErrorKind, LoopRunContext};

use crate::subagent::{
    directions::direction_prompt,
    flavors::{SubagentFlavorId, lookup_flavor, parse_flavor_id},
    gate_resolution::BoundedSubagentGateResolutionStore,
    goal_store::{SubagentGoalStore, SubagentGoalStoreError},
};

pub struct RebornSubagentPromptMaterialSource<G>
where
    G: SubagentGoalStore + ?Sized,
{
    goal_store: Arc<G>,
    flavor_id: SubagentFlavorId,
}

impl<G> RebornSubagentPromptMaterialSource<G>
where
    G: SubagentGoalStore + ?Sized,
{
    pub fn new(goal_store: Arc<G>, flavor_id: SubagentFlavorId) -> Self {
        Self {
            goal_store,
            flavor_id,
        }
    }
}

pub struct GateBackedSubagentPromptMaterialSource<G>
where
    G: SubagentGoalStore + ?Sized,
{
    goal_store: Arc<G>,
    gate_store: Arc<BoundedSubagentGateResolutionStore>,
    thread_service: Arc<dyn SessionThreadService>,
}

impl<G> GateBackedSubagentPromptMaterialSource<G>
where
    G: SubagentGoalStore + ?Sized,
{
    pub fn new(
        goal_store: Arc<G>,
        gate_store: Arc<BoundedSubagentGateResolutionStore>,
        thread_service: Arc<dyn SessionThreadService>,
    ) -> Self {
        Self {
            goal_store,
            gate_store,
            thread_service,
        }
    }
}

#[async_trait]
impl<G> SubagentPromptMaterialSource for GateBackedSubagentPromptMaterialSource<G>
where
    G: SubagentGoalStore + Send + Sync + ?Sized,
{
    async fn material_for_run(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<SubagentPromptMaterial, AgentLoopHostError> {
        let flavor_id = self
            .gate_store
            .subagent_kind_for_child(run_context.run_id)
            .map_err(|error| AgentLoopHostError::new(error.kind, error.safe_summary))?;
        let flavor_id = match flavor_id {
            Some(flavor_id) => flavor_id,
            None => thread_metadata_for_run(self.thread_service.as_ref(), run_context)
                .await?
                .map(|metadata| metadata.subagent_kind)
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "subagent run has no recorded flavor",
                    )
                })?,
        };
        let flavor_id = parse_flavor_id(flavor_id.as_str()).ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "subagent run recorded an unknown flavor",
            )
        })?;
        let goal = goal_for_run(
            self.goal_store.as_ref(),
            Some(self.thread_service.as_ref()),
            run_context,
        )
        .await?;
        material_for_flavor_with_goal(goal, flavor_id)
    }
}

#[async_trait]
impl<G> SubagentPromptMaterialSource for RebornSubagentPromptMaterialSource<G>
where
    G: SubagentGoalStore + Send + Sync + ?Sized,
{
    async fn material_for_run(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<SubagentPromptMaterial, AgentLoopHostError> {
        let goal = goal_for_run(self.goal_store.as_ref(), None, run_context).await?;
        material_for_flavor_with_goal(goal, self.flavor_id)
    }
}

async fn goal_for_run<G>(
    goal_store: &G,
    thread_service: Option<&dyn SessionThreadService>,
    run_context: &LoopRunContext,
) -> Result<SubagentPromptGoal, AgentLoopHostError>
where
    G: SubagentGoalStore + Send + Sync + ?Sized,
{
    match goal_store
        .get_goal(&run_context.scope, run_context.run_id)
        .await
    {
        Ok(goal) => Ok(SubagentPromptGoal {
            task: goal.task,
            handoff: goal.handoff,
        }),
        Err(SubagentGoalStoreError::NotFound { .. }) => {
            let Some(thread_service) = thread_service else {
                return Err(map_goal_error(SubagentGoalStoreError::NotFound {
                    run_id: run_context.run_id,
                }));
            };
            goal_from_thread(thread_service, run_context).await
        }
        Err(error) => Err(map_goal_error(error)),
    }
}

fn material_for_flavor_with_goal(
    goal: SubagentPromptGoal,
    flavor_id: SubagentFlavorId,
) -> Result<SubagentPromptMaterial, AgentLoopHostError> {
    let flavor = lookup_flavor(flavor_id).ok_or_else(|| {
        AgentLoopHostError::new(AgentLoopHostErrorKind::Invalid, "unknown subagent flavor")
    })?;
    let allowed_capabilities = flavor
        .tool_allowlist
        .iter()
        .map(|id| CapabilityId::new(id.as_str()))
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Invalid,
                format!("invalid subagent capability allowlist: {error}"),
            )
        })?;
    Ok(SubagentPromptMaterial {
        direction_markdown: direction_prompt(flavor.direction).to_string(),
        goal,
        allowed_capabilities,
    })
}

async fn goal_from_thread(
    thread_service: &dyn SessionThreadService,
    run_context: &LoopRunContext,
) -> Result<SubagentPromptGoal, AgentLoopHostError> {
    let thread_scope = thread_scope_for_run(run_context)?;
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: run_context.thread_id.clone(),
        })
        .await
        .map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("subagent thread history unavailable: {error}"),
            )
        })?;
    let metadata = history
        .thread
        .metadata_json
        .as_deref()
        .and_then(parse_subagent_thread_metadata);
    let metadata = metadata.and_then(|metadata| metadata.handoff);
    let task = history
        .messages
        .iter()
        .find(|message| {
            message.kind == MessageKind::User
                && matches!(
                    message.status,
                    MessageStatus::Submitted | MessageStatus::Finalized
                )
        })
        .and_then(|message| message.content.clone())
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "subagent run has no persisted goal message",
            )
        })?;
    Ok(SubagentPromptGoal {
        task: strip_persisted_handoff(&task, metadata.as_deref()).to_string(),
        handoff: metadata,
    })
}

fn strip_persisted_handoff<'a>(task: &'a str, handoff: Option<&str>) -> &'a str {
    let Some(handoff) = handoff else {
        return task;
    };
    let suffix = format!("\n\nParent handoff:\n{handoff}");
    if let Some(stripped) = task.strip_suffix(&suffix) {
        return stripped;
    }
    let sanitized_suffix = format!(" Parent handoff: {handoff}");
    task.strip_suffix(&sanitized_suffix).unwrap_or(task)
}

async fn thread_metadata_for_run(
    thread_service: &dyn SessionThreadService,
    run_context: &LoopRunContext,
) -> Result<Option<SubagentThreadMetadata>, AgentLoopHostError> {
    let thread_scope = thread_scope_for_run(run_context)?;
    thread_service
        .read_thread(ThreadHistoryRequest {
            scope: thread_scope,
            thread_id: run_context.thread_id.clone(),
        })
        .await
        .map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("subagent thread metadata unavailable: {error}"),
            )
        })
        .map(|thread| {
            thread
                .metadata_json
                .as_deref()
                .and_then(parse_subagent_thread_metadata)
        })
}

fn parse_subagent_thread_metadata(raw: &str) -> Option<SubagentThreadMetadata> {
    serde_json::from_str::<SubagentThreadMetadata>(raw)
        .ok()
        .filter(|metadata| metadata.kind == SubagentThreadKind::Subagent)
}

fn thread_scope_for_run(run_context: &LoopRunContext) -> Result<ThreadScope, AgentLoopHostError> {
    let agent_id = run_context.scope.agent_id.clone().ok_or_else(|| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "subagent run scope is missing agent id",
        )
    })?;
    Ok(ThreadScope {
        tenant_id: run_context.scope.tenant_id.clone(),
        agent_id,
        project_id: run_context.scope.project_id.clone(),
        owner_user_id: run_context
            .actor
            .as_ref()
            .map(|actor| actor.user_id.clone()),
        mission_id: None,
    })
}

fn map_goal_error(error: SubagentGoalStoreError) -> AgentLoopHostError {
    let kind = match error {
        SubagentGoalStoreError::NotFound { .. } => AgentLoopHostErrorKind::InvalidInvocation,
        SubagentGoalStoreError::PayloadTooLarge { .. } => AgentLoopHostErrorKind::BudgetExceeded,
        SubagentGoalStoreError::DuplicateKey { .. } => AgentLoopHostErrorKind::InvalidInvocation,
        SubagentGoalStoreError::Backend { .. } => AgentLoopHostErrorKind::Unavailable,
    };
    AgentLoopHostError::new(kind, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_host_api::{AgentId, CapabilityId, TenantId, ThreadId};
    use ironclaw_loop_support::{
        AwaitedChildSetRecord, SpawnSubagentMode, SubagentGateResolutionStore, SubagentKindId,
    };
    use ironclaw_threads::{
        AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageContent,
    };
    use ironclaw_turns::{
        GateRef, LoopResultRef, ReplyTargetBindingRef, SourceBindingRef, TurnRunId, TurnScope,
    };

    use crate::subagent::{
        flavors::SubagentFlavorId,
        goal_store::{InMemoryBoundedSubagentGoalStore, SubagentGoal},
    };

    use super::*;

    #[tokio::test]
    async fn material_source_fails_loud_on_goal_miss() {
        let store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let source = RebornSubagentPromptMaterialSource::new(store, SubagentFlavorId::General);
        let context = ironclaw_agent_loop::test_support::test_run_context("missing-goal");

        let error = source.material_for_run(&context).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[tokio::test]
    async fn material_source_combines_static_direction_goal_and_allowlist() {
        let store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let context = ironclaw_agent_loop::test_support::test_run_context("goal");
        store
            .put_goal(
                &context.scope,
                context.run_id,
                SubagentGoal {
                    task: "research task".to_string(),
                    handoff: Some("handoff".to_string()),
                },
            )
            .await
            .unwrap();
        let source = RebornSubagentPromptMaterialSource::new(store, SubagentFlavorId::Researcher);

        let material = source.material_for_run(&context).await.unwrap();

        assert!(material.direction_markdown.contains("research subagent"));
        assert_eq!(material.goal.task, "research task");
        assert!(
            material
                .allowed_capabilities
                .iter()
                .any(|cap| cap.as_str() == ironclaw_host_runtime::HTTP_CAPABILITY_ID)
        );
    }

    #[tokio::test]
    async fn gate_backed_material_source_uses_gate_flavor_and_goal_store() {
        let store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let context = ironclaw_agent_loop::test_support::test_run_context("gate-backed-goal");
        store
            .put_goal(
                &context.scope,
                context.run_id,
                SubagentGoal {
                    task: "research task".to_string(),
                    handoff: None,
                },
            )
            .await
            .unwrap();
        gate_store
            .record_awaited_child(awaited_child_record(
                &context,
                SubagentKindId::new("researcher").unwrap(),
            ))
            .await
            .unwrap();
        let source = GateBackedSubagentPromptMaterialSource::new(
            store,
            gate_store,
            Arc::new(InMemorySessionThreadService::default()),
        );

        let material = source.material_for_run(&context).await.unwrap();

        assert!(material.direction_markdown.contains("research subagent"));
        assert_eq!(material.goal.task, "research task");
    }

    #[tokio::test]
    async fn gate_backed_material_source_falls_back_to_thread_metadata() {
        let store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let mut context = ironclaw_agent_loop::test_support::test_run_context("thread-flavor");
        context.scope.agent_id = Some(AgentId::new("agent-thread-flavor").unwrap());
        ensure_subagent_thread(
            thread_service.as_ref(),
            &context,
            Some(SubagentKindId::new("general").unwrap()),
            Some("task from thread"),
        )
        .await;
        let source = GateBackedSubagentPromptMaterialSource::new(store, gate_store, thread_service);

        let material = source.material_for_run(&context).await.unwrap();

        assert_eq!(material.goal.task, "task from thread");
        assert!(
            material
                .direction_markdown
                .contains("general-purpose subagent")
        );
    }

    #[tokio::test]
    async fn gate_backed_material_source_errors_when_no_flavor_is_recorded() {
        let store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let mut context = ironclaw_agent_loop::test_support::test_run_context("missing-flavor");
        context.scope.agent_id = Some(AgentId::new("agent-missing-flavor").unwrap());
        ensure_subagent_thread(thread_service.as_ref(), &context, None, None).await;
        let source = GateBackedSubagentPromptMaterialSource::new(store, gate_store, thread_service);

        let error = source.material_for_run(&context).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("no recorded flavor"));
    }

    #[tokio::test]
    async fn gate_backed_material_source_errors_when_flavor_is_unknown() {
        let store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let gate_store = Arc::new(BoundedSubagentGateResolutionStore::new());
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let mut context = ironclaw_agent_loop::test_support::test_run_context("unknown-flavor");
        context.scope.agent_id = Some(AgentId::new("agent-unknown-flavor").unwrap());
        ensure_subagent_thread(
            thread_service.as_ref(),
            &context,
            Some(SubagentKindId::new("unknown").unwrap()),
            None,
        )
        .await;
        let source = GateBackedSubagentPromptMaterialSource::new(store, gate_store, thread_service);

        let error = source.material_for_run(&context).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("unknown flavor"));
    }

    #[test]
    fn strip_persisted_handoff_removes_multiline_and_sanitized_suffixes() {
        assert_eq!(
            strip_persisted_handoff("task\n\nParent handoff:\nnotes", Some("notes")),
            "task"
        );
        assert_eq!(
            strip_persisted_handoff("task Parent handoff: notes", Some("notes")),
            "task"
        );
        assert_eq!(
            strip_persisted_handoff("task without handoff", Some("notes")),
            "task without handoff"
        );
        assert_eq!(strip_persisted_handoff("task", None), "task");
    }

    fn awaited_child_record(
        context: &LoopRunContext,
        subagent_kind: SubagentKindId,
    ) -> AwaitedChildSetRecord {
        let tenant = TenantId::new("tenant").unwrap();
        let agent = AgentId::new("agent").unwrap();
        AwaitedChildSetRecord {
            gate_ref: GateRef::new("gate:subagent:prompt-material").unwrap(),
            parent_run_context: context.clone(),
            tree_root_run_id: context.run_id,
            child_scope: TurnScope::new(
                tenant,
                Some(agent),
                None,
                ThreadId::new("child-thread").unwrap(),
            ),
            child_run_id: context.run_id,
            child_thread_id: context.thread_id.clone(),
            source_binding_ref: SourceBindingRef::new("subagent-source:prompt").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("subagent-reply:prompt").unwrap(),
            subagent_kind,
            spawn_capability_id: CapabilityId::new(
                ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
            )
            .unwrap(),
            result_ref: LoopResultRef::new("result:subagent.prompt").unwrap(),
            mode: SpawnSubagentMode::Blocking,
        }
    }

    async fn ensure_subagent_thread(
        thread_service: &InMemorySessionThreadService,
        context: &LoopRunContext,
        subagent_kind: Option<SubagentKindId>,
        message: Option<&str>,
    ) {
        let metadata_json = subagent_kind.map(|subagent_kind| {
            serde_json::to_string(&SubagentThreadMetadata {
                kind: SubagentThreadKind::Subagent,
                parent_run_id: TurnRunId::new(),
                parent_thread_id: ThreadId::new("parent-thread").unwrap(),
                tree_root_run_id: context.run_id,
                child_run_id: context.run_id,
                subagent_kind,
                mode: SpawnSubagentMode::Blocking,
                result_ref: LoopResultRef::new("result:subagent.prompt").unwrap(),
                handoff: None,
            })
            .unwrap()
        });
        let scope = thread_scope_for_run(context).unwrap();
        thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(context.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: None,
                metadata_json,
            })
            .await
            .unwrap();
        if let Some(message) = message {
            let accepted = thread_service
                .accept_inbound_message(AcceptInboundMessageRequest {
                    scope,
                    thread_id: context.thread_id.clone(),
                    actor_id: "test".to_string(),
                    source_binding_id: None,
                    reply_target_binding_id: None,
                    external_event_id: None,
                    content: MessageContent::text(message),
                })
                .await
                .unwrap();
            thread_service
                .mark_message_submitted(
                    &thread_scope_for_run(context).unwrap(),
                    &context.thread_id,
                    accepted.message_id,
                    context.turn_id.to_string(),
                    context.run_id.to_string(),
                )
                .await
                .unwrap();
        }
    }
}
