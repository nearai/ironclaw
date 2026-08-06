use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::ids::{CapabilityId, InvocationId, ProcessId};
use ironclaw_loop_contracts::{AgentLoopHostError, AgentLoopHostErrorKind, LoopRunContext};
use ironclaw_loop_host::{
    SubagentGoalRecord, SubagentPromptGoal, SubagentPromptMaterial, SubagentPromptMaterialSource,
    SubagentThreadKind, SubagentThreadMetadata,
};
use ironclaw_processes::{GetProcessInputRequest, ProcessInputPort};
use ironclaw_threads::{
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};

use crate::subagent::{
    directions::direction_prompt,
    flavors::{SubagentFlavorId, lookup_flavor, parse_flavor_id},
};

#[cfg(test)]
pub struct RebornSubagentPromptMaterialSource {
    process_inputs: Arc<dyn ProcessInputPort<Error = ironclaw_turns::TurnError>>,
    flavor_id: SubagentFlavorId,
}

#[cfg(test)]
impl RebornSubagentPromptMaterialSource {
    pub fn new(
        process_inputs: Arc<dyn ProcessInputPort<Error = ironclaw_turns::TurnError>>,
        flavor_id: SubagentFlavorId,
    ) -> Self {
        Self {
            process_inputs,
            flavor_id,
        }
    }
}

/// §3 replacement inventory: the gate-store read this type was named for
/// (`GateBackedSubagentPromptMaterialSource::material_for_run`'s
/// `.gate_store.subagent_kind_for_child(...)` call) dies outright — the
/// existing thread-metadata fallback below already covers every case the
/// gate read did, per the design doc's verified ruling. The struct keeps its
/// name (a rename would ripple further with no behavior change) but no
/// longer holds a gate/edge dependency at all.
pub struct GateBackedSubagentPromptMaterialSource {
    process_inputs: Arc<dyn ProcessInputPort<Error = ironclaw_turns::TurnError>>,
    thread_service: Arc<dyn SessionThreadService>,
}

impl GateBackedSubagentPromptMaterialSource {
    pub fn new(
        process_inputs: Arc<dyn ProcessInputPort<Error = ironclaw_turns::TurnError>>,
        thread_service: Arc<dyn SessionThreadService>,
    ) -> Self {
        Self {
            process_inputs,
            thread_service,
        }
    }
}

#[async_trait]
impl SubagentPromptMaterialSource for GateBackedSubagentPromptMaterialSource {
    async fn material_for_run(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<SubagentPromptMaterial, AgentLoopHostError> {
        let flavor_id = thread_metadata_for_run(self.thread_service.as_ref(), run_context)
            .await?
            .map(|metadata| metadata.subagent_kind)
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "subagent run has no recorded flavor",
                )
            })?;
        let flavor_id = parse_flavor_id(flavor_id.as_str()).ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "subagent run recorded an unknown flavor",
            )
        })?;
        let goal = goal_for_run(
            self.process_inputs.as_ref(),
            Some(self.thread_service.as_ref()),
            run_context,
        )
        .await?;
        material_for_flavor_with_goal(goal, flavor_id)
    }
}

#[cfg(test)]
#[async_trait]
impl SubagentPromptMaterialSource for RebornSubagentPromptMaterialSource {
    async fn material_for_run(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<SubagentPromptMaterial, AgentLoopHostError> {
        let goal = goal_for_run(self.process_inputs.as_ref(), None, run_context).await?;
        material_for_flavor_with_goal(goal, self.flavor_id)
    }
}

async fn goal_for_run(
    process_inputs: &dyn ProcessInputPort<Error = ironclaw_turns::TurnError>,
    thread_service: Option<&dyn SessionThreadService>,
    run_context: &LoopRunContext,
) -> Result<SubagentPromptGoal, AgentLoopHostError> {
    let mut scope = run_context.scope.to_resource_scope();
    scope.invocation_id = InvocationId::from_uuid(run_context.run_id.as_uuid());
    match process_inputs
        .get_process_input(GetProcessInputRequest {
            process_id: ProcessId::from_uuid(run_context.run_id.as_uuid()),
            scope,
        })
        .await
    {
        Ok(Some(input)) => {
            if input.input_ref.as_str() != "subagent-goal:v1" {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    format!(
                        "subagent run has unsupported process input {}",
                        input.input_ref.as_str()
                    ),
                ));
            }
            let goal = serde_json::from_slice::<SubagentGoalRecord>(input.payload.as_bytes())
                .map_err(|error| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        format!("subagent process input is invalid: {error}"),
                    )
                })?;
            Ok(SubagentPromptGoal {
                task: goal.task,
                handoff: goal.handoff,
            })
        }
        Ok(None) => {
            let Some(thread_service) = thread_service else {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    format!("subagent goal for run {} not found", run_context.run_id),
                ));
            };
            goal_from_thread(thread_service, run_context).await
        }
        Err(error) => Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::Unavailable,
            format!("subagent process input unavailable: {error}"),
        )),
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use ironclaw_host_api::ids::{AgentId, ThreadId};
    use ironclaw_host_api::turn::{LoopResultRef, TurnGateRef, TurnRunId};
    use ironclaw_loop_host::{SpawnSubagentMode, SubagentKindId};
    use ironclaw_processes::{
        GetProcessInputRequest, ProcessInputPayload, ProcessInputPort, ProcessInputRecord,
        ProcessInputRef,
    };
    use ironclaw_threads::{
        AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService,
        MessageContent,
    };

    use crate::subagent::flavors::SubagentFlavorId;

    use super::*;

    struct StaticProcessInput(Option<ProcessInputRecord>);

    #[async_trait]
    impl ProcessInputPort for StaticProcessInput {
        type Error = ironclaw_turns::TurnError;

        async fn get_process_input(
            &self,
            request: GetProcessInputRequest,
        ) -> Result<Option<ProcessInputRecord>, Self::Error> {
            Ok(self
                .0
                .as_ref()
                .filter(|record| {
                    record.process_id == request.process_id && record.scope == request.scope
                })
                .cloned())
        }
    }

    fn process_inputs(
        context: &LoopRunContext,
        goal: Option<SubagentGoalRecord>,
    ) -> Arc<dyn ProcessInputPort<Error = ironclaw_turns::TurnError>> {
        let mut scope = context.scope.to_resource_scope();
        scope.invocation_id = InvocationId::from_uuid(context.run_id.as_uuid());
        Arc::new(StaticProcessInput(goal.map(|goal| {
            ProcessInputRecord {
                process_id: ProcessId::from_uuid(context.run_id.as_uuid()),
                scope,
                input_ref: ProcessInputRef::from_trusted("subagent-goal:v1"),
                payload: ProcessInputPayload::new(
                    serde_json::to_vec(&goal).expect("serialize subagent goal"),
                )
                .expect("bounded subagent goal"),
                created_at: Utc::now(),
            }
        })))
    }

    #[tokio::test]
    async fn material_source_fails_loud_on_goal_miss() {
        let context = ironclaw_agent_loop::test_support::test_run_context("missing-goal");
        let source = RebornSubagentPromptMaterialSource::new(
            process_inputs(&context, None),
            SubagentFlavorId::General,
        );

        let error = source.material_for_run(&context).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[tokio::test]
    async fn material_source_combines_static_direction_goal_and_allowlist() {
        let context = ironclaw_agent_loop::test_support::test_run_context("goal");
        let source = RebornSubagentPromptMaterialSource::new(
            process_inputs(
                &context,
                Some(SubagentGoalRecord {
                    task: "research task".to_string(),
                    handoff: Some("handoff".to_string()),
                }),
            ),
            SubagentFlavorId::Planner,
        );

        let material = source.material_for_run(&context).await.unwrap();

        assert!(material.direction_markdown.contains("planning subagent"));
        assert_eq!(material.goal.task, "research task");
        assert!(
            material
                .allowed_capabilities
                .iter()
                .any(|cap| cap.as_str() == ironclaw_host_runtime::HTTP_CAPABILITY_ID)
        );
    }

    // `gate_backed_material_source_uses_gate_flavor_and_goal_store` deleted:
    // it tested the gate-store-priority-over-thread-metadata path, which
    // dies with this PR's replacement of the gate store (§3 — the existing
    // thread-metadata fallback, exercised by the tests below, already
    // covers every case the deleted gate read did).

    #[tokio::test]
    async fn material_source_uses_thread_metadata_for_flavor() {
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
        let source = GateBackedSubagentPromptMaterialSource::new(
            process_inputs(&context, None),
            thread_service,
        );

        let material = source.material_for_run(&context).await.unwrap();

        assert_eq!(material.goal.task, "task from thread");
        assert!(
            material
                .direction_markdown
                .contains("general-purpose subagent")
        );
    }

    #[tokio::test]
    async fn material_source_errors_when_no_flavor_is_recorded() {
        let thread_service = Arc::new(InMemorySessionThreadService::default());
        let mut context = ironclaw_agent_loop::test_support::test_run_context("missing-flavor");
        context.scope.agent_id = Some(AgentId::new("agent-missing-flavor").unwrap());
        ensure_subagent_thread(thread_service.as_ref(), &context, None, None).await;
        let source = GateBackedSubagentPromptMaterialSource::new(
            process_inputs(&context, None),
            thread_service,
        );

        let error = source.material_for_run(&context).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("no recorded flavor"));
    }

    #[tokio::test]
    async fn material_source_errors_when_flavor_is_unknown() {
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
        let source = GateBackedSubagentPromptMaterialSource::new(
            process_inputs(&context, None),
            thread_service,
        );

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
                spawn_provider_call_id: None,
                handoff: None,
                parent_run_context: context.clone(),
                gate_ref: TurnGateRef::new("gate:subagent-prompt-test").unwrap(),
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
