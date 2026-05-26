use std::{
    collections::HashMap,
    fmt,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{CapabilityId, InvocationId, ThreadId};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, SessionThreadService,
    ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, GateRef, IdempotencyKey, LoopGateRef, LoopResultRef,
    ReplyTargetBindingRef, RunProfileRequest, SanitizedCancelReason, SourceBindingRef,
    SubmitChildRunRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError,
    TurnErrorCategory, TurnRunId, TurnScope, TurnSpawnTreePort, TurnSpawnTreeStateStore,
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

pub const DEFAULT_SUBAGENT_MAX_DEPTH: u32 = 1;
pub const DEFAULT_SUBAGENT_MAX_SPAWN_PER_TURN: u32 = 4;
pub const DEFAULT_SUBAGENT_MAX_TREE_DESCENDANTS: u32 = 16;
pub const DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID: &str = "builtin.spawn_subagent";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SubagentKindId(String);

impl SubagentKindId {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    fn validate(value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Err("subagent kind id cannot be empty".to_string());
        }
        if value.len() > 64 {
            return Err("subagent kind id cannot exceed 64 bytes".to_string());
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(
                "subagent kind id may only contain ascii letters, digits, '_' or '-'".to_string(),
            );
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl AsRef<str> for SubagentKindId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for SubagentKindId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SubagentKindId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

impl TryFrom<String> for SubagentKindId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SubagentKindId> for String {
    fn from(value: SubagentKindId) -> Self {
        value.into_inner()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnSubagentMode {
    Blocking,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnSubagentArgs {
    #[serde(rename = "flavor_id")]
    pub subagent_kind: SubagentKindId,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
    pub mode: SpawnSubagentMode,
}

impl SpawnSubagentArgs {
    pub fn spawn_mode(&self) -> SpawnSubagentMode {
        self.mode
    }
}

#[derive(Debug, Deserialize)]
struct SpawnSubagentWireArgs {
    #[serde(rename = "flavor_id")]
    subagent_kind: SubagentKindId,
    task: String,
    #[serde(default)]
    handoff: Option<String>,
    #[serde(default)]
    mode: Option<SpawnSubagentMode>,
    #[serde(default)]
    run_in_background: bool,
}

impl From<SpawnSubagentWireArgs> for SpawnSubagentArgs {
    fn from(value: SpawnSubagentWireArgs) -> Self {
        let mode = value.mode.unwrap_or({
            if value.run_in_background {
                SpawnSubagentMode::Background
            } else {
                SpawnSubagentMode::Blocking
            }
        });
        Self {
            subagent_kind: value.subagent_kind,
            task: value.task,
            handoff: value.handoff,
            mode,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentDefinition {
    pub subagent_kind: SubagentKindId,
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
    pub tree_root_run_id: TurnRunId,
    pub child_scope: TurnScope,
    pub child_run_id: TurnRunId,
    pub child_thread_id: ThreadId,
    pub source_binding_ref: SourceBindingRef,
    pub reply_target_binding_ref: ReplyTargetBindingRef,
    pub subagent_kind: SubagentKindId,
    pub spawn_capability_id: CapabilityId,
    pub result_ref: LoopResultRef,
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
    #[serde(rename = "flavor")]
    pub subagent_kind: SubagentKindId,
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
pub trait SubagentDefinitionResolver: Send + Sync {
    async fn resolve_kind(
        &self,
        kind: &SubagentKindId,
    ) -> Result<Option<SubagentDefinition>, AgentLoopHostError>;

    async fn definition_of_run(
        &self,
        _run_id: TurnRunId,
    ) -> Result<Option<SubagentDefinition>, AgentLoopHostError> {
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
            max_depth: DEFAULT_SUBAGENT_MAX_DEPTH,
            max_spawn_per_turn: DEFAULT_SUBAGENT_MAX_SPAWN_PER_TURN,
            max_tree_descendants: DEFAULT_SUBAGENT_MAX_TREE_DESCENDANTS,
        }
    }
}

#[derive(Clone)]
pub struct SubagentSpawnDeps {
    pub coordinator: Arc<dyn TurnCoordinator>,
    pub child_runs: Arc<dyn TurnSpawnTreePort>,
    pub turn_state_store: Arc<dyn TurnSpawnTreeStateStore>,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub goal_store: Arc<dyn SubagentSpawnGoalStore>,
    pub gate_store: Arc<dyn SubagentGateResolutionStore>,
    pub definition_resolver: Arc<dyn SubagentDefinitionResolver>,
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
    definition: SubagentDefinition,
    child_scope: ThreadScope,
    child_run_id: TurnRunId,
    tree_root: TurnRunId,
}

#[derive(Default)]
struct SpawnCompensationState {
    goal_written: Option<(TurnScope, TurnRunId)>,
    gate_written: Option<GateRef>,
    result_written: Option<LoopResultRef>,
    submitted_child_tree: Option<(TurnScope, TurnRunId)>,
}

impl SpawnCompensationState {
    async fn rollback(&mut self, deps: &SubagentSpawnDeps, run_context: &LoopRunContext) {
        if let Some(gate_ref) = self.gate_written.as_ref() {
            let _ = deps.gate_store.delete_awaited_child(gate_ref).await;
        }
        if let Some((scope, run_id)) = self.goal_written.as_ref() {
            let _ = deps.goal_store.delete_goal(scope, *run_id).await;
        }
        if let Some(result_ref) = self.result_written.as_ref() {
            let _ = deps
                .result_writer
                .delete_capability_result(run_context, result_ref)
                .await;
        }
        if let Some((scope, tree_root)) = self.submitted_child_tree.as_ref() {
            let _ = deps
                .turn_state_store
                .release_tree_descendants(scope, *tree_root, 1)
                .await;
        }
    }
}

struct SpawnSlotGuard<'a> {
    port: &'a SubagentSpawnCapabilityPort,
    active: bool,
}

impl<'a> SpawnSlotGuard<'a> {
    fn commit(mut self) {
        self.active = false;
    }
}

impl Drop for SpawnSlotGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            self.port.release_spawn_slot();
        }
    }
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

    fn reserve_spawn_slot(&self) -> Option<SpawnSlotGuard<'_>> {
        self.try_reserve_spawn_slot().then_some(SpawnSlotGuard {
            port: self,
            active: true,
        })
    }

    fn release_spawn_slot(&self) {
        let previous =
            self.spawned_this_turn
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    current.checked_sub(1)
                });
        if previous.is_err() {
            tracing::warn!(
                run_id = %self.run_context.run_id,
                spawn_id = %self.spawn_id,
                "subagent spawn slot release ignored because no slot was reserved"
            );
        }
    }

    async fn handle_spawn(
        &self,
        invocation: &CapabilityInvocation,
        args: SpawnSubagentArgs,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let Some(spawn_slot) = self.reserve_spawn_slot() else {
            return Ok(spawn_rejected("fanout_cap_exceeded"));
        };

        let Some(agent_id) = self.run_context.scope.agent_id.clone() else {
            return Ok(spawn_rejected("spawn_requires_agent_scope"));
        };
        let Some(actor) = self.run_context.actor.clone() else {
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
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "parent run record not found for subagent spawn",
                )
            })?;
        let child_depth = parent_record.subagent_depth.saturating_add(1);
        if child_depth > self.limits.max_depth {
            return Ok(spawn_rejected("depth_cap_exceeded"));
        }
        let parent_definition = self
            .deps
            .definition_resolver
            .definition_of_run(self.run_context.run_id)
            .await?;
        match parent_definition {
            Some(parent_definition) if !parent_definition.allow_nesting => {
                return Ok(spawn_rejected("nesting_not_permitted"));
            }
            None if parent_record.subagent_depth > 0 => {
                return Ok(spawn_rejected("nesting_not_permitted"));
            }
            _ => {}
        }

        let resolved = self
            .deps
            .definition_resolver
            .resolve_kind(&args.subagent_kind)
            .await?;
        let Some(definition) = resolved else {
            return Ok(spawn_rejected("unknown_subagent_kind"));
        };

        let child_scope = ThreadScope {
            tenant_id: self.run_context.scope.tenant_id.clone(),
            agent_id,
            project_id: self.run_context.scope.project_id.clone(),
            owner_user_id: Some(owner_user_id.clone()),
            mission_id: None,
        };
        let child_run_id = TurnRunId::new();
        let tree_root = parent_record
            .spawn_tree_root_run_id
            .unwrap_or(self.run_context.run_id);
        let mut compensation = SpawnCompensationState::default();
        let spawn_ctx = SpawnContext {
            definition,
            child_scope,
            child_run_id,
            tree_root,
        };

        let result = self
            .finish_spawn(args, spawn_ctx, actor, invocation, &mut compensation)
            .await;
        match result {
            Ok(outcome) => {
                spawn_slot.commit();
                Ok(outcome)
            }
            Err(error) => {
                compensation
                    .rollback(self.deps.as_ref(), &self.run_context)
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
        invocation: &CapabilityInvocation,
        compensation: &mut SpawnCompensationState,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let SpawnContext {
            definition,
            child_scope,
            child_run_id,
            tree_root,
        } = ctx;
        let child_thread_id =
            ThreadId::new(format!("subagent-{}", child_run_id.as_uuid().simple()))
                .map_err(invalid_static_ref)?;
        let mode = args.spawn_mode();
        // The gate ref is also returned as a `LoopGateRef`; keep the opaque
        // suffix colon-free so it satisfies the model-visible loop ref
        // contract (`gate:<ascii-id>` with only alnum/underscore/dash/dot).
        let gate_ref = GateRef::new(match mode {
            SpawnSubagentMode::Blocking => format!("gate:subagent.{child_run_id}"),
            SpawnSubagentMode::Background => format!("gate:subagent-bg.{child_run_id}"),
        })
        .map_err(invalid_static_ref)?;
        let payload = spawn_result_payload(
            child_run_id,
            &child_thread_id,
            &definition.subagent_kind,
            mode,
            "spawned",
            false,
        );
        let result_ref = self
            .deps
            .result_writer
            .write_capability_result(
                &self.run_context,
                &invocation.input_ref,
                InvocationId::new(),
                &self.spawn_id,
                payload,
            )
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
                    subagent_kind: definition.subagent_kind.clone(),
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
        compensation.goal_written = Some((child_turn_scope.clone(), child_run_id));

        self.deps
            .gate_store
            .record_awaited_child(AwaitedChildSetRecord {
                gate_ref: gate_ref.clone(),
                parent_run_context: self.run_context.clone(),
                tree_root_run_id: tree_root,
                child_scope: child_turn_scope.clone(),
                child_run_id,
                child_thread_id: child_thread.thread_id.clone(),
                source_binding_ref: source_binding_ref(self.run_context.run_id, child_run_id)?,
                reply_target_binding_ref: reply_target_binding_ref(
                    self.run_context.run_id,
                    child_run_id,
                )?,
                subagent_kind: definition.subagent_kind.clone(),
                spawn_capability_id: self.spawn_id.clone(),
                result_ref: result_ref.clone(),
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
            .child_runs
            .submit_child_run(SubmitChildRunRequest {
                parent_scope: self.run_context.scope.clone(),
                parent_run_id: self.run_context.run_id,
                child_scope: child_turn_scope.clone(),
                actor: actor.clone(),
                accepted_message_ref,
                source_binding_ref,
                reply_target_binding_ref,
                requested_run_profile: Some(definition.requested_run_profile),
                idempotency_key,
                received_at: Utc::now(),
                requested_run_id: Some(child_run_id),
                spawn_tree_descendant_cap: self.limits.max_tree_descendants,
            })
            .await
            .map_err(map_turn_error)?;
        compensation.submitted_child_tree = Some((self.run_context.scope.clone(), tree_root));
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
        serde_json::from_value::<SpawnSubagentWireArgs>(value)
            .map(Into::into)
            .map_err(|error| {
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
    subagent_kind: &SubagentKindId,
    mode: SpawnSubagentMode,
    status: &'static str,
    output_available: bool,
) -> serde_json::Value {
    serde_json::json!({
        "child_run_id": child_run_id,
        "child_thread_id": child_thread_id,
        "flavor": subagent_kind,
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
mod tests;
