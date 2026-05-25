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

/// Discriminator for child thread metadata. Single-variant today; the enum
/// shape exists so callers cannot match against a magic `"subagent"` string
/// (see `.claude/rules/types.md` "fixed small sets → enums").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentThreadKind {
    Subagent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentThreadMetadata {
    pub kind: SubagentThreadKind,
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

struct SpawnContext {
    flavor: SubagentFlavorPolicy,
    child_scope: ThreadScope,
    child_run_id: TurnRunId,
    tree_root: TurnRunId,
    child_depth: u32,
}

#[derive(Default)]
struct SpawnCompensationState {
    goal_written: bool,
    gate_written: Option<GateRef>,
    result_written: Option<LoopResultRef>,
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
        self.spawned_this_turn
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                if current < self.limits.max_spawn_per_turn {
                    current.checked_add(1)
                } else {
                    None
                }
            })
            .is_ok()
    }

    fn release_spawn_slot(&self) {
        if self
            .spawned_this_turn
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_sub(1)
            })
            .is_err()
        {
            tracing::warn!("subagent spawn slot release ignored because no slot was reserved");
        }
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
            .map_err(|error| {
                self.release_spawn_slot();
                map_turn_error(error)
            })?
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
        let parent_flavor_outcome = self
            .deps
            .flavor_resolver
            .flavor_of_run(self.run_context.run_id)
            .await
            .inspect_err(|_| {
                self.release_spawn_slot();
            })?;
        match parent_flavor_outcome {
            Some(parent_flavor) if !parent_flavor.allow_nesting => {
                self.release_spawn_slot();
                return Ok(spawn_rejected("nesting_not_permitted"));
            }
            None if parent_record.subagent_depth > 0 => {
                self.release_spawn_slot();
                return Ok(spawn_rejected("nesting_not_permitted"));
            }
            _ => {}
        }

        let resolved = self
            .deps
            .flavor_resolver
            .resolve_flavor(&args.flavor_id)
            .await
            .inspect_err(|_| {
                self.release_spawn_slot();
            })?;
        let Some(flavor) = resolved else {
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
        let mut compensation = SpawnCompensationState::default();
        let spawn_ctx = SpawnContext {
            flavor,
            child_scope,
            child_run_id,
            tree_root,
            child_depth,
        };

        let result = self
            .finish_spawn(args, spawn_ctx, actor, &mut compensation)
            .await;
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.release_spawn_slot();
                if let Some(gate_ref) = compensation.gate_written.as_ref() {
                    let _ = self.deps.gate_store.delete_awaited_child(gate_ref).await;
                }
                if compensation.goal_written {
                    let _ = self
                        .deps
                        .goal_store
                        .delete_goal(&child_turn_scope, child_run_id)
                        .await;
                }
                if let Some(result_ref) = compensation.result_written.as_ref() {
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
        let auth_input_ref = {
            let auth_input_refs = self.auth_input_refs.lock().map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "subagent spawn authorization input store is unavailable",
                )
            })?;
            auth_input_refs.get(&invocation.input_ref).cloned()
        };
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
                self.remove_auth_input_ref(&invocation.input_ref)?;
                Ok(None)
            }
            other if other.is_suspension() => Ok(Some(other)),
            other => {
                self.remove_auth_input_ref(&invocation.input_ref)?;
                Ok(Some(other))
            }
        }
    }

    fn remove_auth_input_ref(
        &self,
        input_ref: &CapabilityInputRef,
    ) -> Result<(), AgentLoopHostError> {
        self.auth_input_refs
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "subagent spawn authorization input store is unavailable",
                )
            })?
            .remove(input_ref);
        Ok(())
    }

    async fn finish_spawn(
        &self,
        args: SpawnSubagentArgs,
        ctx: SpawnContext,
        actor: TurnActor,
        compensation: &mut SpawnCompensationState,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let SpawnContext {
            flavor,
            child_scope,
            child_run_id,
            tree_root,
            child_depth,
        } = ctx;
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
        compensation.result_written = Some(result_ref.clone());
        let child_thread = self
            .deps
            .thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: child_scope.clone(),
                thread_id: Some(child_thread_id.clone()),
                created_by_actor_id: format!("subagent:{}", self.run_context.run_id),
                title: Some("Subagent".to_string()),
                metadata_json: Some(child_thread_metadata(SubagentThreadMetadata {
                    kind: SubagentThreadKind::Subagent,
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
        compensation.goal_written = true;

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
        compensation.gate_written = Some(gate_ref.clone());

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
    use chrono::Utc;
    use ironclaw_host_api::{AgentId, TenantId, UserId};
    use ironclaw_threads::InMemorySessionThreadService;
    use ironclaw_turns::{
        AcceptedMessageRef, CancelRunResponse, EventCursor, GetRunStateRequest,
        InMemoryRunProfileResolver, ResumeTurnRequest, ResumeTurnResponse,
        RunProfileResolutionRequest, RunProfileResolver, SpawnTreeReservation, TurnId,
        TurnRunProfile, TurnRunRecord, TurnRunState, TurnStatus,
        run_profile::{CapabilityResultMessage, CapabilitySurfaceVersion},
    };
    use serde_json::json;

    use super::*;

    struct StaticInputResolver {
        value: Result<serde_json::Value, AgentLoopHostError>,
    }

    struct StaticSpawnInputCodec {
        args: SpawnSubagentArgs,
    }

    struct StaticFlavorResolver {
        resolved: Option<SubagentFlavorPolicy>,
        parent: Option<SubagentFlavorPolicy>,
    }

    struct AuthPassPort;

    struct NoopResultWriter;

    struct NoopGoalStore;

    struct StaticCoordinator;

    struct StaticTurnStateStore {
        record: Option<TurnRunRecord>,
    }

    #[async_trait]
    impl LoopCapabilityInputResolver for StaticInputResolver {
        async fn resolve_capability_input(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<serde_json::Value, AgentLoopHostError> {
            self.value.clone()
        }
    }

    #[async_trait]
    impl SpawnSubagentInputCodec for StaticSpawnInputCodec {
        async fn decode(
            &self,
            _run_context: &LoopRunContext,
            _input_ref: &CapabilityInputRef,
        ) -> Result<SpawnSubagentArgs, AgentLoopHostError> {
            Ok(self.args.clone())
        }
    }

    #[async_trait]
    impl SubagentFlavorPolicyResolver for StaticFlavorResolver {
        async fn resolve_flavor(
            &self,
            _flavor_id: &str,
        ) -> Result<Option<SubagentFlavorPolicy>, AgentLoopHostError> {
            Ok(self.resolved.clone())
        }

        async fn flavor_of_run(
            &self,
            _run_id: TurnRunId,
        ) -> Result<Option<SubagentFlavorPolicy>, AgentLoopHostError> {
            Ok(self.parent.clone())
        }
    }

    #[async_trait]
    impl LoopCapabilityPort for AuthPassPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
                descriptors: Vec::new(),
            })
        }

        async fn invoke_capability(
            &self,
            _request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new("result:auth").unwrap(),
                safe_summary: "authorized".to_string(),
                terminate_hint: false,
            }))
        }

        async fn invoke_capability_batch(
            &self,
            _request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            unreachable!("batch auth is not used by these tests")
        }
    }

    #[async_trait]
    impl LoopCapabilityResultWriter for NoopResultWriter {
        async fn write_capability_result(
            &self,
            _run_context: &LoopRunContext,
            _capability_id: &CapabilityId,
            _output: serde_json::Value,
        ) -> Result<LoopResultRef, AgentLoopHostError> {
            Ok(LoopResultRef::new("result:spawn").unwrap())
        }
    }

    #[async_trait]
    impl SubagentSpawnGoalStore for NoopGoalStore {
        async fn put_goal(
            &self,
            _scope: &TurnScope,
            _run_id: TurnRunId,
            _goal: SubagentGoalRecord,
        ) -> Result<(), AgentLoopHostError> {
            Ok(())
        }

        async fn delete_goal(
            &self,
            _scope: &TurnScope,
            _run_id: TurnRunId,
        ) -> Result<(), AgentLoopHostError> {
            Ok(())
        }
    }

    #[async_trait]
    impl TurnCoordinator for StaticCoordinator {
        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            unreachable!("spawn early-return tests do not submit child turns")
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            unreachable!("spawn tests do not resume turns")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unreachable!("spawn tests do not cancel turns")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            unreachable!("spawn tests do not read run state through coordinator")
        }
    }

    #[async_trait]
    impl TurnStateStore for StaticTurnStateStore {
        async fn submit_turn(
            &self,
            _request: SubmitTurnRequest,
            _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
            _run_profile_resolver: &dyn RunProfileResolver,
        ) -> Result<SubmitTurnResponse, TurnError> {
            unreachable!("spawn tests do not submit through state store")
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            unreachable!("spawn tests do not resume through state store")
        }

        async fn request_cancel(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            unreachable!("spawn tests do not cancel through state store")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            unreachable!("spawn tests do not get run state")
        }

        async fn children_of(
            &self,
            _scope: &TurnScope,
            _run_id: TurnRunId,
        ) -> Result<Vec<TurnRunRecord>, TurnError> {
            Ok(Vec::new())
        }

        async fn get_run_record(
            &self,
            _scope: &TurnScope,
            _run_id: TurnRunId,
        ) -> Result<Option<TurnRunRecord>, TurnError> {
            Ok(self.record.clone())
        }

        async fn reserve_tree_descendants(
            &self,
            scope: &TurnScope,
            root_run_id: TurnRunId,
            delta: u32,
            _cap: u32,
        ) -> Result<SpawnTreeReservation, TurnError> {
            Ok(SpawnTreeReservation {
                scope: scope.clone(),
                root_run_id,
                descendant_count: u64::from(delta),
            })
        }

        async fn release_tree_descendants(
            &self,
            _scope: &TurnScope,
            _root_run_id: TurnRunId,
            _delta: u32,
        ) -> Result<(), TurnError> {
            Ok(())
        }
    }

    fn input_ref() -> CapabilityInputRef {
        CapabilityInputRef::new("input:spawn").unwrap()
    }

    async fn test_run_context(label: &str) -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        LoopRunContext::new(
            TurnScope::new(
                TenantId::new(format!("tenant-{label}")).unwrap(),
                None,
                None,
                ThreadId::new(format!("thread-{label}")).unwrap(),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        )
    }

    async fn test_run_context_with_agent_actor(label: &str) -> LoopRunContext {
        let mut context = test_run_context(label).await.with_actor(TurnActor::new(
            UserId::new(format!("user-{label}")).unwrap(),
        ));
        context.scope.agent_id = Some(AgentId::new(format!("agent-{label}")).unwrap());
        context
    }

    fn default_spawn_args() -> SpawnSubagentArgs {
        SpawnSubagentArgs {
            flavor_id: "general".to_string(),
            task: "task".to_string(),
            handoff: None,
            mode: None,
            run_in_background: false,
        }
    }

    fn flavor_policy(allow_nesting: bool) -> SubagentFlavorPolicy {
        SubagentFlavorPolicy {
            flavor_id: "general".to_string(),
            allow_nesting,
            requested_run_profile: RunProfileRequest::new("subagent-test").unwrap(),
        }
    }

    fn turn_record(run_context: &LoopRunContext, subagent_depth: u32) -> TurnRunRecord {
        let lineage_root = (subagent_depth > 0).then(TurnRunId::new);
        TurnRunRecord {
            run_id: run_context.run_id,
            turn_id: run_context.turn_id,
            scope: run_context.scope.clone(),
            accepted_message_ref: AcceptedMessageRef::new("msg:parent").unwrap(),
            source_binding_ref: SourceBindingRef::new("source:parent").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:parent").unwrap(),
            status: TurnStatus::Queued,
            profile: TurnRunProfile::from_resolved(run_context.resolved_run_profile.clone()),
            resolved_model_route: None,
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(1),
            runner_id: None,
            lease_token: None,
            lease_expires_at: None,
            last_heartbeat_at: None,
            claim_count: 0,
            received_at: Utc::now(),
            parent_run_id: lineage_root,
            subagent_depth,
            spawn_tree_root_run_id: lineage_root,
        }
    }

    async fn spawn_test_port(
        run_context: LoopRunContext,
        limits: SubagentSpawnLimits,
        parent_subagent_depth: Option<u32>,
        resolver: StaticFlavorResolver,
    ) -> SubagentSpawnCapabilityPort {
        let turn_store = Arc::new(StaticTurnStateStore {
            record: parent_subagent_depth.map(|depth| turn_record(&run_context, depth)),
        });
        let coordinator: Arc<dyn TurnCoordinator> = Arc::new(StaticCoordinator);
        let deps = Arc::new(SubagentSpawnDeps {
            coordinator,
            turn_state_store: turn_store,
            thread_service: Arc::new(InMemorySessionThreadService::default()),
            goal_store: Arc::new(NoopGoalStore),
            gate_store: Arc::new(InMemorySubagentGateResolutionStore::default()),
            flavor_resolver: Arc::new(resolver),
            spawn_input_codec: Arc::new(StaticSpawnInputCodec {
                args: default_spawn_args(),
            }),
            result_writer: Arc::new(NoopResultWriter),
        });
        let port = SubagentSpawnCapabilityPort::new(
            Arc::new(AuthPassPort),
            run_context,
            CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            limits,
            deps,
        );
        port.auth_input_refs
            .lock()
            .unwrap()
            .insert(input_ref(), CapabilityInputRef::new("input:auth").unwrap());
        port
    }

    async fn invoke_spawn(port: &SubagentSpawnCapabilityPort) -> CapabilityOutcome {
        port.invoke_capability(CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
            capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
            input_ref: input_ref(),
        })
        .await
        .unwrap()
    }

    fn denied_reason(outcome: CapabilityOutcome) -> String {
        let CapabilityOutcome::Denied(denied) = outcome else {
            panic!("expected denied outcome");
        };
        denied.reason_kind.as_str().to_string()
    }

    #[test]
    fn spawn_mode_prefers_explicit_mode_over_legacy_background_flag() {
        let mut args = SpawnSubagentArgs {
            flavor_id: "general".to_string(),
            task: "task".to_string(),
            handoff: None,
            mode: None,
            run_in_background: false,
        };
        assert_eq!(args.spawn_mode(), SpawnSubagentMode::Blocking);

        args.run_in_background = true;
        assert_eq!(args.spawn_mode(), SpawnSubagentMode::Background);

        args.mode = Some(SpawnSubagentMode::Blocking);
        assert_eq!(args.spawn_mode(), SpawnSubagentMode::Blocking);

        args.mode = Some(SpawnSubagentMode::Background);
        args.run_in_background = false;
        assert_eq!(args.spawn_mode(), SpawnSubagentMode::Background);
    }

    #[tokio::test]
    async fn invoke_spawn_rejects_when_fanout_cap_is_exceeded() {
        let context = test_run_context_with_agent_actor("spawn-fanout").await;
        let port = spawn_test_port(
            context,
            SubagentSpawnLimits {
                max_spawn_per_turn: 0,
                ..SubagentSpawnLimits::default()
            },
            Some(0),
            StaticFlavorResolver {
                resolved: Some(flavor_policy(false)),
                parent: None,
            },
        )
        .await;

        assert_eq!(
            denied_reason(invoke_spawn(&port).await),
            "fanout_cap_exceeded"
        );
    }

    #[tokio::test]
    async fn invoke_spawn_rejects_missing_agent_scope() {
        let mut context = test_run_context_with_agent_actor("spawn-agent-scope").await;
        context.scope.agent_id = None;
        let port = spawn_test_port(
            context,
            SubagentSpawnLimits::default(),
            None,
            StaticFlavorResolver {
                resolved: Some(flavor_policy(false)),
                parent: None,
            },
        )
        .await;

        assert_eq!(
            denied_reason(invoke_spawn(&port).await),
            "spawn_requires_agent_scope"
        );
    }

    #[tokio::test]
    async fn invoke_spawn_rejects_missing_actor() {
        let mut context = test_run_context("spawn-actor").await;
        context.scope.agent_id = Some(AgentId::new("agent-spawn-actor").unwrap());
        let port = spawn_test_port(
            context,
            SubagentSpawnLimits::default(),
            None,
            StaticFlavorResolver {
                resolved: Some(flavor_policy(false)),
                parent: None,
            },
        )
        .await;

        assert_eq!(
            denied_reason(invoke_spawn(&port).await),
            "spawn_requires_actor"
        );
    }

    #[tokio::test]
    async fn invoke_spawn_fails_when_parent_record_is_missing() {
        let context = test_run_context_with_agent_actor("spawn-parent-missing").await;
        let port = spawn_test_port(
            context,
            SubagentSpawnLimits::default(),
            None,
            StaticFlavorResolver {
                resolved: Some(flavor_policy(false)),
                parent: None,
            },
        )
        .await;

        let error = port
            .invoke_capability(CapabilityInvocation {
                surface_version: CapabilitySurfaceVersion::new("surface:test").unwrap(),
                capability_id: CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
                input_ref: input_ref(),
            })
            .await
            .unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert!(error.safe_summary.contains("parent run record not found"));
    }

    #[tokio::test]
    async fn invoke_spawn_rejects_depth_cap() {
        let context = test_run_context_with_agent_actor("spawn-depth").await;
        let port = spawn_test_port(
            context,
            SubagentSpawnLimits {
                max_depth: 1,
                ..SubagentSpawnLimits::default()
            },
            Some(1),
            StaticFlavorResolver {
                resolved: Some(flavor_policy(false)),
                parent: Some(flavor_policy(true)),
            },
        )
        .await;

        assert_eq!(
            denied_reason(invoke_spawn(&port).await),
            "depth_cap_exceeded"
        );
    }

    #[tokio::test]
    async fn invoke_spawn_rejects_subagent_parent_without_resolved_parent_flavor() {
        let context = test_run_context_with_agent_actor("spawn-nesting").await;
        let port = spawn_test_port(
            context,
            SubagentSpawnLimits {
                max_depth: 2,
                ..SubagentSpawnLimits::default()
            },
            Some(1),
            StaticFlavorResolver {
                resolved: Some(flavor_policy(false)),
                parent: None,
            },
        )
        .await;

        assert_eq!(
            denied_reason(invoke_spawn(&port).await),
            "nesting_not_permitted"
        );
    }

    #[tokio::test]
    async fn json_spawn_input_codec_decodes_legacy_background_flag() {
        let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
            value: Ok(json!({
                "flavor_id": "general",
                "task": "investigate",
                "run_in_background": true
            })),
        }));
        let context = test_run_context("spawn-codec").await;

        let args = codec.decode(&context, &input_ref()).await.unwrap();

        assert_eq!(args.flavor_id, "general");
        assert_eq!(args.task, "investigate");
        assert_eq!(args.spawn_mode(), SpawnSubagentMode::Background);
    }

    #[tokio::test]
    async fn json_spawn_input_codec_rejects_invalid_shape() {
        let context = test_run_context("spawn-codec-invalid").await;
        for value in [
            json!({"task": "missing flavor"}),
            json!({"flavor_id": "general", "task": 42}),
            json!({"flavor_id": "general", "task": "task", "mode": "later"}),
        ] {
            let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
                value: Ok(value),
            }));

            let error = codec.decode(&context, &input_ref()).await.unwrap_err();

            assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
            assert!(error.safe_summary.contains("invalid spawn_subagent input"));
        }
    }

    #[tokio::test]
    async fn json_spawn_input_codec_propagates_resolver_error() {
        let codec = JsonSpawnSubagentInputCodec::new(Arc::new(StaticInputResolver {
            value: Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "input unavailable",
            )),
        }));
        let context = test_run_context("spawn-codec-error").await;

        let error = codec.decode(&context, &input_ref()).await.unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
        assert_eq!(error.safe_summary, "input unavailable");
    }

    #[test]
    fn spawn_rejected_preserves_spawn_specific_reason_kind() {
        let CapabilityOutcome::Denied(denied) = spawn_rejected("depth_cap_exceeded") else {
            panic!("spawn_rejected should deny");
        };

        assert_eq!(denied.reason_kind.as_str(), "depth_cap_exceeded");
        assert!(denied.safe_summary.contains("depth_cap_exceeded"));
    }

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
