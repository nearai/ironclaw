use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{CapabilityId, ThreadId};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, SessionThreadService,
    ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, GateRef, IdempotencyKey, LoopGateRef, LoopResultRef,
    ReplyTargetBindingRef, RunProfileRequest, SanitizedCancelReason, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError,
    TurnErrorCategory, TurnRunId, TurnScope, TurnStateStore,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityCallCandidate, CapabilityDenied,
        CapabilityDeniedReasonKind, CapabilityInputRef, CapabilityInvocation, CapabilityOutcome,
        LoopCapabilityPort, LoopRunContext, LoopSafeSummary, ProviderToolCall,
        ProviderToolDefinition, VisibleCapabilityRequest, VisibleCapabilitySurface,
        sanitize_model_visible_text,
    },
};
use serde::{Deserialize, Serialize};

use crate::{LoopCapabilityInputResolver, LoopCapabilityResultWriter};

pub const DEFAULT_MAX_SUBAGENT_DEPTH: u32 = 1;
pub const DEFAULT_MAX_SPAWN_PER_TURN: u32 = 4;
pub const DEFAULT_MAX_TREE_DESCENDANTS: u32 = 16;
pub const DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID: &str = "builtin.spawn_subagent";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnSubagentMode {
    Blocking,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnSubagentArgs {
    pub flavor_id: String,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
    #[serde(default)]
    pub mode: Option<SpawnSubagentMode>,
    #[serde(default)]
    pub run_in_background: bool,
}

impl SpawnSubagentArgs {
    pub fn spawn_mode(&self) -> SpawnSubagentMode {
        self.mode.unwrap_or({
            if self.run_in_background {
                SpawnSubagentMode::Background
            } else {
                SpawnSubagentMode::Blocking
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentFlavorPolicy {
    pub flavor_id: String,
    pub allow_nesting: bool,
    pub requested_run_profile: RunProfileRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentGoalRecord {
    pub task: String,
    pub handoff: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitedChildSetRecord {
    pub gate_ref: GateRef,
    pub parent_run_context: LoopRunContext,
    pub parent_scope: TurnScope,
    pub parent_run_id: TurnRunId,
    pub tree_root_run_id: TurnRunId,
    pub child_scope: TurnScope,
    pub child_run_id: TurnRunId,
    pub child_thread_id: ThreadId,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub flavor_id: String,
    pub spawn_capability_id: CapabilityId,
    pub background_result_ref: Option<LoopResultRef>,
    pub mode: SpawnSubagentMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentThreadMetadata {
    pub kind: String,
    pub parent_run_id: TurnRunId,
    pub parent_thread_id: ThreadId,
    pub tree_root_run_id: TurnRunId,
    pub child_run_id: TurnRunId,
    pub flavor: String,
    pub mode: SpawnSubagentMode,
    pub result_ref: LoopResultRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}

#[async_trait]
pub trait SpawnSubagentInputCodec: Send + Sync {
    async fn decode(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<SpawnSubagentArgs, AgentLoopHostError>;

    async fn register_provider_tool_call_input(
        &self,
        _run_context: &LoopRunContext,
        _tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "spawn_subagent provider tool-call input registration is not supported",
        ))
    }
}

#[async_trait]
pub trait SubagentFlavorPolicyResolver: Send + Sync {
    async fn resolve_flavor(
        &self,
        flavor_id: &str,
    ) -> Result<Option<SubagentFlavorPolicy>, AgentLoopHostError>;

    async fn flavor_of_run(
        &self,
        _run_id: TurnRunId,
    ) -> Result<Option<SubagentFlavorPolicy>, AgentLoopHostError> {
        Ok(None)
    }
}

#[async_trait]
pub trait SubagentSpawnGoalStore: Send + Sync {
    async fn put_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        goal: SubagentGoalRecord,
    ) -> Result<(), AgentLoopHostError>;

    async fn delete_goal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<(), AgentLoopHostError>;
}

#[async_trait]
pub trait SubagentGateResolutionStore: Send + Sync {
    async fn record_awaited_child(
        &self,
        record: AwaitedChildSetRecord,
    ) -> Result<(), AgentLoopHostError>;

    async fn delete_awaited_child(&self, gate_ref: &GateRef) -> Result<(), AgentLoopHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentSpawnLimits {
    pub max_depth: u32,
    pub max_spawn_per_turn: u32,
    pub max_tree_descendants: u32,
}

impl Default for SubagentSpawnLimits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_SUBAGENT_DEPTH,
            max_spawn_per_turn: DEFAULT_MAX_SPAWN_PER_TURN,
            max_tree_descendants: DEFAULT_MAX_TREE_DESCENDANTS,
        }
    }
}

#[derive(Clone)]
pub struct SubagentSpawnDeps {
    pub coordinator: Arc<dyn TurnCoordinator>,
    pub turn_state_store: Arc<dyn TurnStateStore>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub goal_store: Arc<dyn SubagentSpawnGoalStore>,
    pub gate_store: Arc<dyn SubagentGateResolutionStore>,
    pub flavor_resolver: Arc<dyn SubagentFlavorPolicyResolver>,
    pub spawn_input_codec: Arc<dyn SpawnSubagentInputCodec>,
    pub result_writer: Arc<dyn LoopCapabilityResultWriter>,
}

pub struct SubagentSpawnCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    spawn_id: CapabilityId,
    limits: SubagentSpawnLimits,
    deps: Arc<SubagentSpawnDeps>,
    auth_input_refs: Mutex<HashMap<CapabilityInputRef, CapabilityInputRef>>,
    spawned_this_turn: AtomicU32,
}

impl SubagentSpawnCapabilityPort {
    pub fn new(
        inner: Arc<dyn LoopCapabilityPort>,
        run_context: LoopRunContext,
        spawn_id: CapabilityId,
        limits: SubagentSpawnLimits,
        deps: Arc<SubagentSpawnDeps>,
    ) -> Self {
        Self {
            inner,
            run_context,
            spawn_id,
            limits,
            deps,
            auth_input_refs: Mutex::new(HashMap::new()),
            spawned_this_turn: AtomicU32::new(0),
        }
    }

    fn is_spawn(&self, capability_id: &CapabilityId) -> bool {
        capability_id == &self.spawn_id
    }

    fn try_reserve_spawn_slot(&self) -> bool {
        let mut current = self.spawned_this_turn.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(1) else {
                return false;
            };
            if next > self.limits.max_spawn_per_turn {
                return false;
            }
            match self.spawned_this_turn.compare_exchange(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(observed) => current = observed,
            }
        }
    }

    fn release_spawn_slot(&self) {
        let _ =
            self.spawned_this_turn
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    current.checked_sub(1)
                });
    }

    async fn handle_spawn(
        &self,
        _invocation: &CapabilityInvocation,
        args: SpawnSubagentArgs,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if !self.try_reserve_spawn_slot() {
            return Ok(spawn_rejected("fanout_cap_exceeded"));
        }

        let Some(agent_id) = self.run_context.scope.agent_id.clone() else {
            self.release_spawn_slot();
            return Ok(spawn_rejected("spawn_requires_agent_scope"));
        };
        let Some(actor) = self.run_context.actor.clone() else {
            self.release_spawn_slot();
            return Ok(spawn_rejected("spawn_requires_actor"));
        };
        let owner_user_id = actor.user_id.clone();
        let parent_record = self
            .deps
            .turn_state_store
            .get_run_record(&self.run_context.scope, self.run_context.run_id)
            .await
            .map_err(map_turn_error)?
            .ok_or_else(|| {
                self.release_spawn_slot();
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "parent run record not found for subagent spawn",
                )
            })?;
        let child_depth = parent_record.subagent_depth.saturating_add(1);
        if child_depth > self.limits.max_depth {
            self.release_spawn_slot();
            return Ok(spawn_rejected("depth_cap_exceeded"));
        }
        if let Some(parent_flavor) = self
            .deps
            .flavor_resolver
            .flavor_of_run(self.run_context.run_id)
            .await?
            && !parent_flavor.allow_nesting
        {
            self.release_spawn_slot();
            return Ok(spawn_rejected("nesting_not_permitted"));
        }

        let Some(flavor) = self
            .deps
            .flavor_resolver
            .resolve_flavor(&args.flavor_id)
            .await?
        else {
            self.release_spawn_slot();
            return Ok(spawn_rejected("unknown_flavor"));
        };

        let child_scope = ThreadScope {
            tenant_id: self.run_context.scope.tenant_id.clone(),
            agent_id,
            project_id: self.run_context.scope.project_id.clone(),
            owner_user_id: Some(owner_user_id.clone()),
            mission_id: None,
        };
        let child_turn_scope = TurnScope::new(
            child_scope.tenant_id.clone(),
            Some(child_scope.agent_id.clone()),
            child_scope.project_id.clone(),
            ThreadId::new(format!(
                "subagent-pending-{}",
                TurnRunId::new().as_uuid().simple()
            ))
            .map_err(invalid_static_ref)?,
        );
        let child_run_id = self
            .deps
            .coordinator
            .prepare_turn(child_turn_scope.clone())
            .await
            .map_err(|error| {
                self.release_spawn_slot();
                map_turn_error(error)
            })?;
        let tree_root = parent_record
            .spawn_tree_root_run_id
            .unwrap_or(self.run_context.run_id);
        self.deps
            .turn_state_store
            .reserve_tree_descendants(
                &self.run_context.scope,
                tree_root,
                1,
                self.limits.max_tree_descendants,
            )
            .await
            .map_err(|error| {
                self.release_spawn_slot();
                map_reservation_error(error)
            })?;
        let mut goal_written = false;
        let mut gate_written: Option<GateRef> = None;
        let mut result_written: Option<LoopResultRef> = None;

        let result = self
            .finish_spawn(
                args,
                flavor,
                child_scope,
                child_run_id,
                tree_root,
                child_depth,
                actor,
                &mut goal_written,
                &mut gate_written,
                &mut result_written,
            )
            .await;
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.release_spawn_slot();
                if let Some(gate_ref) = gate_written.as_ref() {
                    let _ = self.deps.gate_store.delete_awaited_child(gate_ref).await;
                }
                if goal_written {
                    let _ = self
                        .deps
                        .goal_store
                        .delete_goal(&child_turn_scope, child_run_id)
                        .await;
                }
                if let Some(result_ref) = result_written.as_ref() {
                    let _ = self
                        .deps
                        .result_writer
                        .delete_capability_result(&self.run_context, result_ref)
                        .await;
                }
                let _ = self
                    .deps
                    .turn_state_store
                    .release_tree_descendants(&self.run_context.scope, tree_root, 1)
                    .await;
                Err(error)
            }
        }
    }

    async fn authorize_spawn(
        &self,
        invocation: &CapabilityInvocation,
    ) -> Result<Option<CapabilityOutcome>, AgentLoopHostError> {
        let auth_input_ref = self
            .auth_input_refs
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "subagent spawn authorization input store is unavailable",
                )
            })?
            .get(&invocation.input_ref)
            .cloned();
        let Some(auth_input_ref) = auth_input_ref else {
            return Ok(Some(spawn_rejected("spawn_requires_provider_registration")));
        };
        let mut auth_invocation = invocation.clone();
        auth_invocation.input_ref = auth_input_ref;
        match self.inner.invoke_capability(auth_invocation).await? {
            CapabilityOutcome::Completed(result) => {
                let _ = self
                    .deps
                    .result_writer
                    .delete_capability_result(&self.run_context, &result.result_ref)
                    .await;
                self.auth_input_refs
                    .lock()
                    .map_err(|_| {
                        AgentLoopHostError::new(
                            AgentLoopHostErrorKind::Unavailable,
                            "subagent spawn authorization input store is unavailable",
                        )
                    })?
                    .remove(&invocation.input_ref);
                Ok(None)
            }
            other if other.is_suspension() => Ok(Some(other)),
            other => {
                self.auth_input_refs
                    .lock()
                    .map_err(|_| {
                        AgentLoopHostError::new(
                            AgentLoopHostErrorKind::Unavailable,
                            "subagent spawn authorization input store is unavailable",
                        )
                    })?
                    .remove(&invocation.input_ref);
                Ok(Some(other))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_spawn(
        &self,
        args: SpawnSubagentArgs,
        flavor: SubagentFlavorPolicy,
        child_scope: ThreadScope,
        child_run_id: TurnRunId,
        tree_root: TurnRunId,
        child_depth: u32,
        actor: TurnActor,
        goal_written: &mut bool,
        gate_written: &mut Option<GateRef>,
        result_written: &mut Option<LoopResultRef>,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let child_thread_id =
            ThreadId::new(format!("subagent-{}", child_run_id.as_uuid().simple()))
                .map_err(invalid_static_ref)?;
        let mode = args.spawn_mode();
        let gate_ref = GateRef::new(match mode {
            SpawnSubagentMode::Blocking => format!("gate:subagent:{child_run_id}"),
            SpawnSubagentMode::Background => format!("gate:subagent-bg:{child_run_id}"),
        })
        .map_err(invalid_static_ref)?;
        let payload = spawn_result_payload(
            child_run_id,
            &child_thread_id,
            &flavor.flavor_id,
            mode,
            "spawned",
            false,
        );
        let result_ref = self
            .deps
            .result_writer
            .write_capability_result(&self.run_context, &self.spawn_id, payload)
            .await?;
        *result_written = Some(result_ref.clone());
        let child_thread = self
            .deps
            .thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: child_scope.clone(),
                thread_id: Some(child_thread_id.clone()),
                created_by_actor_id: format!("subagent:{}", self.run_context.run_id),
                title: Some("Subagent".to_string()),
                metadata_json: Some(child_thread_metadata(SubagentThreadMetadata {
                    kind: "subagent".to_string(),
                    parent_run_id: self.run_context.run_id,
                    parent_thread_id: self.run_context.thread_id.clone(),
                    tree_root_run_id: tree_root,
                    child_run_id,
                    flavor: flavor.flavor_id.clone(),
                    mode,
                    result_ref: result_ref.clone(),
                    handoff: args.handoff.clone(),
                })?),
            })
            .await
            .map_err(map_thread_error)?;
        let child_turn_scope = TurnScope::new(
            child_scope.tenant_id.clone(),
            Some(child_scope.agent_id.clone()),
            child_scope.project_id.clone(),
            child_thread.thread_id.clone(),
        );
        self.deps
            .goal_store
            .put_goal(
                &child_turn_scope,
                child_run_id,
                SubagentGoalRecord {
                    task: args.task.clone(),
                    handoff: args.handoff.clone(),
                },
            )
            .await?;
        *goal_written = true;

        self.deps
            .gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref: gate_ref.clone(),
                parent_run_context: self.run_context.clone(),
                parent_scope: self.run_context.scope.clone(),
                parent_run_id: self.run_context.run_id,
                tree_root_run_id: tree_root,
                child_scope: child_turn_scope.clone(),
                child_run_id,
                child_thread_id: child_thread.thread_id.clone(),
                source_binding_ref: source_binding_ref(self.run_context.run_id, child_run_id)?,
                reply_target_binding_ref: reply_target_binding_ref(
                    self.run_context.run_id,
                    child_run_id,
                )?,
                flavor_id: flavor.flavor_id.clone(),
                spawn_capability_id: self.spawn_id.clone(),
                background_result_ref: Some(result_ref.clone()),
                mode,
            })
            .await?;
        *gate_written = Some(gate_ref.clone());

        let accepted = self
            .deps
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: child_scope.clone(),
                thread_id: child_thread.thread_id.clone(),
                actor_id: actor.user_id.as_str().to_string(),
                source_binding_id: Some(format!("subagent-source:{child_run_id}")),
                reply_target_binding_id: Some(format!("subagent-reply:{child_run_id}")),
                external_event_id: Some(format!("subagent-spawn:{child_run_id}")),
                content: MessageContent::text(sanitize_model_visible_text(child_initial_message(
                    &args,
                ))),
            })
            .await
            .map_err(map_thread_error)?;
        let accepted_message_ref = accepted_message_ref(accepted.message_id)?;
        let source_binding_ref = source_binding_ref(self.run_context.run_id, child_run_id)?;
        let reply_target_binding_ref =
            reply_target_binding_ref(self.run_context.run_id, child_run_id)?;
        let idempotency_key = idempotency_key(self.run_context.run_id, child_run_id)?;

        let SubmitTurnResponse::Accepted {
            turn_id, run_id, ..
        } = self
            .deps
            .coordinator
            .submit_turn(SubmitTurnRequest {
                scope: child_turn_scope.clone(),
                actor: actor.clone(),
                accepted_message_ref,
                source_binding_ref,
                reply_target_binding_ref,
                requested_run_profile: Some(flavor.requested_run_profile),
                idempotency_key,
                received_at: Utc::now(),
                requested_run_id: Some(child_run_id),
                parent_run_id: Some(self.run_context.run_id),
                subagent_depth: child_depth,
                spawn_tree_root_run_id: Some(tree_root),
            })
            .await
            .map_err(map_turn_error)?;
        if let Err(error) = self
            .deps
            .thread_service
            .mark_message_submitted(
                &child_scope,
                &child_thread.thread_id,
                accepted.message_id,
                turn_id.to_string(),
                run_id.to_string(),
            )
            .await
        {
            self.cancel_child_after_submission_failure(&child_turn_scope, actor, run_id)
                .await?;
            return Err(map_thread_error(error));
        }

        match mode {
            SpawnSubagentMode::Blocking => {
                let loop_gate_ref =
                    LoopGateRef::new(gate_ref.as_str()).map_err(invalid_static_ref)?;
                Ok(CapabilityOutcome::AwaitDependentRun {
                    gate_ref: loop_gate_ref,
                    result_ref,
                    safe_summary: safe_summary("subagent spawned; waiting for completion"),
                })
            }
            SpawnSubagentMode::Background => Ok(CapabilityOutcome::SpawnedChildRun {
                child_run_id,
                result_ref,
                safe_summary: safe_summary("subagent spawned in background"),
            }),
        }
    }

    async fn cancel_child_after_submission_failure(
        &self,
        child_scope: &TurnScope,
        actor: TurnActor,
        child_run_id: TurnRunId,
    ) -> Result<(), AgentLoopHostError> {
        self.deps
            .turn_state_store
            .request_cancel(CancelRunRequest {
                scope: child_scope.clone(),
                actor,
                run_id: child_run_id,
                reason: SanitizedCancelReason::Superseded,
                idempotency_key: IdempotencyKey::new(format!(
                    "subagent-cancel:{}:{}",
                    self.run_context.run_id, child_run_id
                ))
                .map_err(invalid_static_ref)?,
            })
            .await
            .map(|_| ())
            .map_err(map_turn_error)
    }
}

#[async_trait]
impl LoopCapabilityPort for SubagentSpawnCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.inner.tool_definitions()
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let inner_candidate = self
            .inner
            .register_provider_tool_call(tool_call.clone())
            .await?;
        if inner_candidate.capability_id == self.spawn_id {
            let input_ref = self
                .deps
                .spawn_input_codec
                .register_provider_tool_call_input(&self.run_context, &tool_call)
                .await?;
            self.auth_input_refs
                .lock()
                .map_err(|_| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Unavailable,
                        "subagent spawn authorization input store is unavailable",
                    )
                })?
                .insert(input_ref.clone(), inner_candidate.input_ref);
            return Ok(CapabilityCallCandidate {
                input_ref,
                ..inner_candidate
            });
        }
        Ok(inner_candidate)
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.inner.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if self.is_spawn(&request.capability_id) {
            if let Some(outcome) = self.authorize_spawn(&request).await? {
                return Ok(outcome);
            }
            let args = self
                .deps
                .spawn_input_codec
                .decode(&self.run_context, &request.input_ref)
                .await?;
            return self.handle_spawn(&request, args).await;
        }
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::with_capacity(request.invocations.len());
        let mut index = 0_usize;
        while index < request.invocations.len() {
            let invocation = &request.invocations[index];
            if self.is_spawn(&invocation.capability_id) {
                let outcome = if let Some(outcome) = self.authorize_spawn(invocation).await? {
                    outcome
                } else {
                    let args = self
                        .deps
                        .spawn_input_codec
                        .decode(&self.run_context, &invocation.input_ref)
                        .await?;
                    self.handle_spawn(invocation, args).await?
                };
                let suspended = outcome.is_suspension();
                outcomes.push(outcome);
                if suspended && request.stop_on_first_suspension {
                    return Ok(CapabilityBatchOutcome {
                        outcomes,
                        stopped_on_suspension: true,
                    });
                }
                index += 1;
                continue;
            }

            let start = index;
            while index < request.invocations.len()
                && !self.is_spawn(&request.invocations[index].capability_id)
            {
                index += 1;
            }
            let inner = self
                .inner
                .invoke_capability_batch(CapabilityBatchInvocation {
                    invocations: request.invocations[start..index].to_vec(),
                    stop_on_first_suspension: request.stop_on_first_suspension,
                })
                .await?;
            let stopped = inner.stopped_on_suspension;
            outcomes.extend(inner.outcomes);
            if stopped && request.stop_on_first_suspension {
                return Ok(CapabilityBatchOutcome {
                    outcomes,
                    stopped_on_suspension: true,
                });
            }
        }

        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension: false,
        })
    }
}

pub struct JsonSpawnSubagentInputCodec {
    input_resolver: Arc<dyn LoopCapabilityInputResolver>,
}

impl JsonSpawnSubagentInputCodec {
    pub fn new(input_resolver: Arc<dyn LoopCapabilityInputResolver>) -> Self {
        Self { input_resolver }
    }
}

#[async_trait]
impl SpawnSubagentInputCodec for JsonSpawnSubagentInputCodec {
    async fn decode(
        &self,
        run_context: &LoopRunContext,
        input_ref: &CapabilityInputRef,
    ) -> Result<SpawnSubagentArgs, AgentLoopHostError> {
        let value = self
            .input_resolver
            .resolve_capability_input(run_context, input_ref)
            .await?;
        serde_json::from_value(value).map_err(|error| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                format!("invalid spawn_subagent input: {error}"),
            )
        })
    }

    async fn register_provider_tool_call_input(
        &self,
        run_context: &LoopRunContext,
        tool_call: &ProviderToolCall,
    ) -> Result<CapabilityInputRef, AgentLoopHostError> {
        self.input_resolver
            .register_provider_tool_call_input(run_context, tool_call)
            .await
    }
}

#[derive(Default)]
pub struct InMemorySubagentGateResolutionStore {
    inner: parking_lot::Mutex<HashMap<GateRef, AwaitedChildSetRecord>>,
}

impl InMemorySubagentGateResolutionStore {
    pub fn records(&self) -> Vec<AwaitedChildSetRecord> {
        self.inner.lock().values().cloned().collect()
    }
}

#[async_trait]
impl SubagentGateResolutionStore for InMemorySubagentGateResolutionStore {
    async fn record_awaited_child(
        &self,
        record: AwaitedChildSetRecord,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.lock().insert(record.gate_ref.clone(), record);
        Ok(())
    }

    async fn delete_awaited_child(&self, gate_ref: &GateRef) -> Result<(), AgentLoopHostError> {
        self.inner.lock().remove(gate_ref);
        Ok(())
    }
}

fn spawn_rejected(reason: &'static str) -> CapabilityOutcome {
    CapabilityOutcome::Denied(CapabilityDenied {
        reason_kind: CapabilityDeniedReasonKind::unknown(reason)
            .unwrap_or(CapabilityDeniedReasonKind::EmptySurface),
        safe_summary: format!("subagent spawn rejected: {reason}"),
    })
}

fn map_turn_error(error: TurnError) -> AgentLoopHostError {
    let kind = match error.category() {
        TurnErrorCategory::Unauthorized => AgentLoopHostErrorKind::Unauthorized,
        TurnErrorCategory::InvalidRequest => AgentLoopHostErrorKind::InvalidInvocation,
        TurnErrorCategory::CapacityExceeded => AgentLoopHostErrorKind::BudgetExceeded,
        TurnErrorCategory::Unavailable => AgentLoopHostErrorKind::Unavailable,
        TurnErrorCategory::ScopeNotFound
        | TurnErrorCategory::ThreadBusy
        | TurnErrorCategory::AdmissionRejected
        | TurnErrorCategory::Conflict => AgentLoopHostErrorKind::InvalidInvocation,
    };
    AgentLoopHostError::new(kind, error.to_string())
}

fn map_reservation_error(error: TurnError) -> AgentLoopHostError {
    if matches!(error.category(), TurnErrorCategory::CapacityExceeded) {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::BudgetExceeded,
            "subagent spawn rejected: tree_descendant_cap_exceeded",
        )
    } else {
        map_turn_error(error)
    }
}

fn map_thread_error(error: ironclaw_threads::SessionThreadError) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        format!("subagent thread operation failed: {error}"),
    )
}

fn invalid_static_ref(reason: impl ToString) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, reason.to_string())
}

fn child_thread_metadata(metadata: SubagentThreadMetadata) -> Result<String, AgentLoopHostError> {
    serde_json::to_string(&metadata).map_err(|error| {
        AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, error.to_string())
    })
}

fn child_initial_message(args: &SpawnSubagentArgs) -> String {
    let mut message = args.task.clone();
    if let Some(handoff) = args.handoff.as_deref() {
        message.push_str("\n\nParent handoff:\n");
        message.push_str(handoff);
    }
    message
}

fn accepted_message_ref(
    message_id: ThreadMessageId,
) -> Result<AcceptedMessageRef, AgentLoopHostError> {
    AcceptedMessageRef::new(format!("msg:{message_id}")).map_err(invalid_static_ref)
}

fn source_binding_ref(
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
) -> Result<SourceBindingRef, AgentLoopHostError> {
    SourceBindingRef::new(format!("subagent-source:{parent_run_id}:{child_run_id}"))
        .map_err(invalid_static_ref)
}

fn reply_target_binding_ref(
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
) -> Result<ReplyTargetBindingRef, AgentLoopHostError> {
    ReplyTargetBindingRef::new(format!("subagent-reply:{parent_run_id}:{child_run_id}"))
        .map_err(invalid_static_ref)
}

fn idempotency_key(
    parent_run_id: TurnRunId,
    child_run_id: TurnRunId,
) -> Result<IdempotencyKey, AgentLoopHostError> {
    IdempotencyKey::new(format!("subagent-submit:{parent_run_id}:{child_run_id}"))
        .map_err(invalid_static_ref)
}

fn safe_summary(value: &'static str) -> String {
    LoopSafeSummary::new(value)
        .map(|summary| summary.as_str().to_string())
        .unwrap_or_else(|_| value.to_string())
}

fn spawn_result_payload(
    child_run_id: TurnRunId,
    child_thread_id: &ThreadId,
    flavor_id: &str,
    mode: SpawnSubagentMode,
    status: &'static str,
    output_available: bool,
) -> serde_json::Value {
    serde_json::json!({
        "child_run_id": child_run_id,
        "child_thread_id": child_thread_id,
        "flavor": flavor_id,
        "mode": mode_label(mode),
        "status": status,
        "output_available": output_available
    })
}

fn mode_label(mode: SpawnSubagentMode) -> &'static str {
    match mode {
        SpawnSubagentMode::Blocking => "blocking",
        SpawnSubagentMode::Background => "background",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_submit_bindings_are_unique_per_prepared_child_run() {
        let parent_run_id = TurnRunId::new();
        let first_child = TurnRunId::new();
        let second_child = TurnRunId::new();

        assert_ne!(
            source_binding_ref(parent_run_id, first_child).unwrap(),
            source_binding_ref(parent_run_id, second_child).unwrap()
        );
        assert_ne!(
            reply_target_binding_ref(parent_run_id, first_child).unwrap(),
            reply_target_binding_ref(parent_run_id, second_child).unwrap()
        );
        assert_ne!(
            idempotency_key(parent_run_id, first_child).unwrap(),
            idempotency_key(parent_run_id, second_child).unwrap()
        );
    }
}
