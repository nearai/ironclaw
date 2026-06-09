use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{CapabilityId, InvocationId, RuntimeKind, ThreadId};
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
        CapabilityDeniedReasonKind, CapabilityDescriptorView, CapabilityInputRef,
        CapabilityInvocation, CapabilityOutcome, ConcurrencyHint, LoopCapabilityPort,
        LoopRunContext, LoopSafeSummary, ProviderToolCall, ProviderToolCallCapabilityIds,
        ProviderToolCallReplay, ProviderToolDefinition, VisibleCapabilityRequest,
        VisibleCapabilitySurface, sanitize_model_visible_text,
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    CapabilityResultWrite, LoopCapabilityInputResolver, LoopCapabilityResultWriter,
    subagent_prompt_port::DEFAULT_SUBAGENT_GOAL_MAX_BYTES,
};

pub const DEFAULT_SUBAGENT_MAX_DEPTH: u32 = 1;
pub const DEFAULT_SUBAGENT_MAX_SPAWN_PER_TURN: u32 = 4;
pub const DEFAULT_SUBAGENT_MAX_TREE_DESCENDANTS: u32 = 16;
pub const DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID: &str = "builtin.spawn_subagent";
const SPAWN_SUBAGENT_PROVIDER_TOOL_NAME: &str = "builtin__spawn_subagent";
pub(crate) const SPAWN_SUBAGENT_DESCRIPTION: &str =
    include_str!("../prompts/spawn_subagent_description.md");

/// A flavor descriptor passed into [`SubagentSpawnCapabilityPort`] at
/// construction time so the port can build a dynamic `subagent_type` enum
/// schema. Carries a validated [`SubagentKindId`] and a human-readable
/// description bullet used in the schema's `description` field.
#[derive(Clone, Debug)]
pub struct SpawnSubagentFlavorDescriptor {
    pub id: SubagentKindId,
    pub summary: String,
}

pub fn build_spawn_subagent_parameters_schema(
    catalog: &[SpawnSubagentFlavorDescriptor],
) -> serde_json::Value {
    let enum_values: Vec<serde_json::Value> =
        catalog.iter().map(|f| serde_json::json!(f.id)).collect();

    let description = if catalog.is_empty() {
        "Which subagent profile to spawn.".to_string()
    } else {
        let lines: Vec<String> = catalog
            .iter()
            .map(|f| format!("- {}: {}", f.id, f.summary))
            .collect();
        format!(
            "Which subagent profile to spawn. Options:\n{}",
            lines.join("\n")
        )
    };

    let mut subagent_type_props = serde_json::json!({
        "type": "string",
        "description": description,
    });
    if !enum_values.is_empty() {
        subagent_type_props["enum"] = serde_json::Value::Array(enum_values);
    }

    serde_json::json!({
        "type": "object",
        "required": ["subagent_type", "task"],
        "additionalProperties": false,
        "properties": {
            "subagent_type": subagent_type_props,
            "task": {
                "type": "string",
                "maxLength": DEFAULT_SUBAGENT_GOAL_MAX_BYTES,
                "description": "Self-contained task for the child subagent run. Runtime enforces a UTF-8 byte budget; maxLength is a provider-facing character-count hint."
            },
            "handoff": {
                "type": "string",
                "maxLength": DEFAULT_SUBAGENT_GOAL_MAX_BYTES,
                "description": "Optional context appended to the child subagent prompt. Runtime enforces a UTF-8 byte budget; maxLength is a provider-facing character-count hint."
            }
        }
    })
}

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
    #[serde(rename = "subagent_type", alias = "flavor_id")]
    pub subagent_kind: SubagentKindId,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnSubagentWireArgs {
    #[serde(rename = "subagent_type", alias = "flavor_id")]
    subagent_kind: SubagentKindId,
    task: String,
    #[serde(default)]
    handoff: Option<String>,
    #[serde(default)]
    mode: Option<SpawnSubagentWireMode>,
    #[serde(default)]
    run_in_background: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SpawnSubagentWireMode {
    Blocking,
    Background,
}

impl TryFrom<SpawnSubagentWireArgs> for SpawnSubagentArgs {
    type Error = AgentLoopHostError;

    fn try_from(value: SpawnSubagentWireArgs) -> Result<Self, Self::Error> {
        if value.run_in_background {
            return Err(background_subagents_disabled());
        }
        if value.mode == Some(SpawnSubagentWireMode::Background) {
            return Err(background_subagents_disabled());
        }
        if value.task.len() > DEFAULT_SUBAGENT_GOAL_MAX_BYTES {
            return Err(spawn_goal_field_too_large("task", value.task.len()));
        }
        if let Some(handoff) = value.handoff.as_deref()
            && handoff.len() > DEFAULT_SUBAGENT_GOAL_MAX_BYTES
        {
            return Err(spawn_goal_field_too_large("handoff", handoff.len()));
        }
        Ok(Self {
            subagent_kind: value.subagent_kind,
            task: value.task,
            handoff: value.handoff,
        })
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
    parameters_schema: Arc<serde_json::Value>,
    auth_input_refs: Mutex<HashSet<CapabilityInputRef>>,
    spawned_this_turn: AtomicU32,
}

struct SpawnContext {
    definition: SubagentDefinition,
    child_scope: ThreadScope,
    child_run_id: TurnRunId,
    tree_root: TurnRunId,
    gate_override: Option<GateRef>,
}

#[derive(Default)]
struct SpawnCompensationState {
    goal_written: Option<(TurnScope, TurnRunId)>,
    gate_written: Option<GateRef>,
    result_written: Option<LoopResultRef>,
    submitted_child_tree: Option<(TurnScope, TurnRunId)>,
    submitted_child_run: Option<(TurnScope, TurnActor, TurnRunId)>,
    thread_written: Option<(ThreadScope, ThreadId)>,
    spawn_slot_committed: bool,
}

impl SpawnCompensationState {
    async fn rollback(&mut self, deps: &SubagentSpawnDeps, run_context: &LoopRunContext) {
        if let Some((scope, actor, run_id)) = self.submitted_child_run.as_ref() {
            match IdempotencyKey::new(format!(
                "subagent-rollback-cancel:{}:{}",
                run_context.run_id, run_id
            )) {
                Ok(idempotency_key) => {
                    let _ = deps
                        .turn_state_store
                        .request_cancel(CancelRunRequest {
                            scope: scope.clone(),
                            actor: actor.clone(),
                            run_id: *run_id,
                            reason: SanitizedCancelReason::Superseded,
                            idempotency_key,
                        })
                        .await;
                }
                Err(reason) => {
                    tracing::warn!(
                        run_id = %run_context.run_id,
                        child_run_id = %run_id,
                        %reason,
                        "subagent rollback skipped child-run cancel because idempotency key was invalid"
                    );
                }
            }
        }
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
        if let Some((scope, thread_id)) = self.thread_written.as_ref() {
            let _ = deps.thread_service.delete_thread(scope, thread_id).await;
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
        flavor_catalog: Vec<SpawnSubagentFlavorDescriptor>,
    ) -> Self {
        let parameters_schema = Arc::new(build_spawn_subagent_parameters_schema(&flavor_catalog));
        Self {
            inner,
            run_context,
            spawn_id,
            limits,
            deps,
            parameters_schema,
            auth_input_refs: Mutex::new(HashSet::new()),
            spawned_this_turn: AtomicU32::new(0),
        }
    }

    /// Creates a port with a precomputed parameters schema, avoiding the
    /// schema-build cost when the caller already computed it (e.g. the
    /// decorator precomputes once at startup and reuses across `decorate()`
    /// calls). Takes `Arc<serde_json::Value>` so `decorate()` can pass
    /// `Arc::clone(&self.parameters_schema)` — a cheap ref-count bump — rather
    /// than deep-cloning the JSON tree on every loop run.
    pub fn new_with_schema(
        inner: Arc<dyn LoopCapabilityPort>,
        run_context: LoopRunContext,
        spawn_id: CapabilityId,
        limits: SubagentSpawnLimits,
        deps: Arc<SubagentSpawnDeps>,
        parameters_schema: Arc<serde_json::Value>,
    ) -> Self {
        Self {
            inner,
            run_context,
            spawn_id,
            limits,
            deps,
            parameters_schema,
            auth_input_refs: Mutex::new(HashSet::new()),
            spawned_this_turn: AtomicU32::new(0),
        }
    }

    fn is_spawn(&self, capability_id: &CapabilityId) -> bool {
        capability_id == &self.spawn_id
    }

    fn is_spawn_provider_tool_name(&self, tool_name: &str) -> bool {
        tool_name == SPAWN_SUBAGENT_PROVIDER_TOOL_NAME
    }

    fn spawn_tool_definition(&self) -> ProviderToolDefinition {
        ProviderToolDefinition {
            capability_id: self.spawn_id.clone(),
            name: SPAWN_SUBAGENT_PROVIDER_TOOL_NAME.to_string(),
            description: SPAWN_SUBAGENT_DESCRIPTION.to_string(),
            parameters: (*self.parameters_schema).clone(),
        }
    }

    fn spawn_descriptor(&self) -> CapabilityDescriptorView {
        CapabilityDescriptorView {
            capability_id: self.spawn_id.clone(),
            provider: None,
            runtime: RuntimeKind::FirstParty,
            safe_name: self.spawn_id.as_str().to_string(),
            safe_description: SPAWN_SUBAGENT_DESCRIPTION.to_string(),
            concurrency_hint: ConcurrencyHint::Exclusive,
            parameters_schema: (*self.parameters_schema).clone(),
        }
    }

    fn validate_spawn_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        serde_json::from_value::<SpawnSubagentWireArgs>(tool_call.arguments.clone())
            .map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    format!("invalid spawn_subagent input: {error}"),
                )
            })?
            .try_into()
            .map(|_: SpawnSubagentArgs| ())
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

    async fn handle_spawn_with_gate(
        &self,
        invocation: &CapabilityInvocation,
        args: SpawnSubagentArgs,
        gate_override: Option<GateRef>,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let mut compensation = SpawnCompensationState::default();
        self.handle_spawn_with_gate_recording(invocation, args, gate_override, &mut compensation)
            .await
    }

    async fn handle_spawn_with_gate_recording(
        &self,
        invocation: &CapabilityInvocation,
        args: SpawnSubagentArgs,
        gate_override: Option<GateRef>,
        compensation: &mut SpawnCompensationState,
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
        let spawn_ctx = SpawnContext {
            definition,
            child_scope,
            child_run_id,
            tree_root,
            gate_override,
        };

        let result = self
            .finish_spawn(args, spawn_ctx, actor, invocation, compensation)
            .await;
        match result {
            Ok(outcome) => {
                spawn_slot.commit();
                compensation.spawn_slot_committed = true;
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
        let is_registered = {
            let auth_input_refs = self.auth_input_refs.lock().map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "subagent spawn authorization input store is unavailable",
                )
            })?;
            auth_input_refs.contains(&invocation.input_ref)
        };
        if !is_registered {
            return Ok(Some(spawn_rejected("spawn_requires_provider_registration")));
        }
        self.remove_auth_input_ref(&invocation.input_ref)?;
        Ok(None)
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
            gate_override,
        } = ctx;
        let child_thread_id =
            ThreadId::new(format!("subagent-{}", child_run_id.as_uuid().simple()))
                .map_err(invalid_static_ref)?;
        let mode = SpawnSubagentMode::Blocking;
        // The gate ref is also returned as a `LoopGateRef`; keep the opaque
        // suffix colon-free so it satisfies the model-visible loop ref
        // contract (`gate:<ascii-id>` with only alnum/underscore/dash/dot).
        let gate_ref = if let Some(gate_ref) = gate_override {
            gate_ref
        } else {
            GateRef::new(format!("gate:subagent-{child_run_id}")).map_err(invalid_static_ref)?
        };
        let payload = spawn_result_payload(
            child_run_id,
            &child_thread_id,
            &definition.subagent_kind,
            mode,
            "spawned",
            false,
        );
        let (result_ref, byte_len) = self
            .deps
            .result_writer
            .write_capability_result(CapabilityResultWrite {
                run_context: &self.run_context,
                input_ref: &invocation.input_ref,
                invocation_id: InvocationId::new(),
                capability_id: &self.spawn_id,
                output: payload,
                display_preview: None,
            })
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
        compensation.thread_written = Some((child_scope.clone(), child_thread.thread_id.clone()));
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
        compensation.submitted_child_run = Some((child_turn_scope.clone(), actor.clone(), run_id));
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
            return Err(map_thread_error(error));
        }

        let loop_gate_ref = LoopGateRef::new(gate_ref.as_str()).map_err(invalid_static_ref)?;
        Ok(CapabilityOutcome::AwaitDependentRun {
            gate_ref: loop_gate_ref,
            result_ref,
            safe_summary: safe_summary("subagent spawned; waiting for completion"),
            byte_len,
        })
    }

    async fn rollback_batch_compensation(&self, compensations: &mut Vec<SpawnCompensationState>) {
        // Roll back in reverse creation order. Each compensation performs
        // destructive best-effort cleanup, so keeping this serial preserves a
        // deterministic unwind order across child runs and their resources.
        while let Some(mut compensation) = compensations.pop() {
            let release_spawn_slot = compensation.spawn_slot_committed;
            compensation
                .rollback(self.deps.as_ref(), &self.run_context)
                .await;
            if release_spawn_slot {
                self.release_spawn_slot();
            }
        }
    }
}

#[async_trait]
impl LoopCapabilityPort for SubagentSpawnCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let mut definitions = self.inner.tool_definitions()?;
        if !definitions
            .iter()
            .any(|definition| definition.capability_id == self.spawn_id)
        {
            definitions.push(self.spawn_tool_definition());
            definitions.sort_by(|left, right| left.name.cmp(&right.name));
        }
        Ok(definitions)
    }

    fn provider_tool_call_capability_ids(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<ProviderToolCallCapabilityIds, AgentLoopHostError> {
        if self.is_spawn_provider_tool_name(&tool_call.name) {
            return Ok(ProviderToolCallCapabilityIds::single(self.spawn_id.clone()));
        }
        self.inner.provider_tool_call_capability_ids(tool_call)
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        if self.is_spawn_provider_tool_name(&tool_call.name) {
            return self.validate_spawn_provider_tool_call(tool_call);
        }
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        if self.is_spawn_provider_tool_name(&tool_call.name) {
            let surface = self
                .inner
                .visible_capabilities(VisibleCapabilityRequest)
                .await?;
            self.validate_spawn_provider_tool_call(&tool_call)?;
            let provider_turn_id = tool_call.turn_id.clone().ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "provider tool call is missing a provider turn id",
                )
            })?;
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
                .insert(input_ref.clone());
            return Ok(CapabilityCallCandidate {
                surface_version: surface.version,
                capability_id: self.spawn_id.clone(),
                effective_capability_ids: vec![self.spawn_id.clone()],
                input_ref,
                provider_replay: Some(ProviderToolCallReplay {
                    provider_id: tool_call.provider_id,
                    provider_model_id: tool_call.provider_model_id,
                    provider_turn_id,
                    provider_call_id: tool_call.id,
                    provider_tool_name: tool_call.name,
                    arguments: tool_call.arguments,
                    response_reasoning: tool_call.response_reasoning,
                    reasoning: tool_call.reasoning,
                    signature: tool_call.signature,
                }),
            });
        }
        let inner_candidate = self.inner.register_provider_tool_call(tool_call).await?;
        Ok(inner_candidate)
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let mut surface = self.inner.visible_capabilities(request).await?;
        if !surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == self.spawn_id)
        {
            surface.descriptors.push(self.spawn_descriptor());
        }
        Ok(surface)
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if self.is_spawn(&request.capability_id) {
            let args = self
                .deps
                .spawn_input_codec
                .decode(&self.run_context, &request.input_ref)
                .await?;
            if let Some(outcome) = self.authorize_spawn(&request).await? {
                return Ok(outcome);
            }
            return self.handle_spawn_with_gate(&request, args, None).await;
        }
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let mut outcomes = Vec::with_capacity(request.invocations.len());
        let mut batch_compensations = Vec::new();
        // Pre-decode every spawn invocation before allocating the shared batch
        // gate. Only batches with at least two valid blocking spawns benefit
        // from gate coalescing; otherwise the gate would be created and never
        // registered, wasting a TurnRunId.
        let mut spawn_args: HashMap<usize, SpawnSubagentArgs> = HashMap::new();
        let mut blocking_count = 0_usize;
        for (idx, invocation) in request.invocations.iter().enumerate() {
            if !self.is_spawn(&invocation.capability_id) {
                continue;
            }
            let args = self
                .deps
                .spawn_input_codec
                .decode(&self.run_context, &invocation.input_ref)
                .await?;
            blocking_count += 1;
            spawn_args.insert(idx, args);
        }
        let batch_blocking_gate = if blocking_count > 1 {
            Some(
                GateRef::new(format!("gate:subagent-batch-{}", TurnRunId::new()))
                    .map_err(invalid_static_ref)?,
            )
        } else {
            None
        };
        let mut index = 0_usize;
        while index < request.invocations.len() {
            let invocation = &request.invocations[index];
            if self.is_spawn(&invocation.capability_id) {
                let outcome = match self.authorize_spawn(invocation).await {
                    Ok(Some(outcome)) => outcome,
                    Ok(None) => {
                        let args = match spawn_args.remove(&index) {
                            Some(args) => args,
                            None => {
                                let error = AgentLoopHostError::new(
                                    AgentLoopHostErrorKind::Invalid,
                                    "subagent spawn args missing from pre-decode pass",
                                );
                                self.rollback_batch_compensation(&mut batch_compensations)
                                    .await;
                                return Err(error);
                            }
                        };
                        let gate_override = batch_blocking_gate.clone();
                        let mut compensation = SpawnCompensationState::default();
                        let outcome = match self
                            .handle_spawn_with_gate_recording(
                                invocation,
                                args,
                                gate_override,
                                &mut compensation,
                            )
                            .await
                        {
                            Ok(result) => result,
                            Err(error) => {
                                self.rollback_batch_compensation(&mut batch_compensations)
                                    .await;
                                return Err(error);
                            }
                        };
                        batch_compensations.push(compensation);
                        outcome
                    }
                    Err(error) => {
                        self.rollback_batch_compensation(&mut batch_compensations)
                            .await;
                        return Err(error);
                    }
                };
                let batch_await_dependent = matches!(
                    &outcome,
                    CapabilityOutcome::AwaitDependentRun { gate_ref, .. }
                        if batch_blocking_gate
                            .as_ref()
                            .is_some_and(|batch_gate| batch_gate == gate_ref)
                );
                let suspended = outcome.is_suspension();
                outcomes.push(outcome);
                if suspended && request.stop_on_first_suspension && !batch_await_dependent {
                    // Suspension is a partial-success boundary, not a failed
                    // batch; prior successful spawns remain committed.
                    batch_compensations.clear();
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
                .await;
            let inner = match inner {
                Ok(inner) => inner,
                Err(error) => {
                    self.rollback_batch_compensation(&mut batch_compensations)
                        .await;
                    return Err(error);
                }
            };
            let stopped = inner.stopped_on_suspension;
            outcomes.extend(inner.outcomes);
            if stopped && request.stop_on_first_suspension {
                // Propagate the inner partial-success stop without rolling back
                // earlier successful spawns from this outer batch.
                batch_compensations.clear();
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
            .map_err(|error| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    format!("invalid spawn_subagent input: {error}"),
                )
            })?
            .try_into()
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

fn background_subagents_disabled() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        "background subagents are disabled pending durable completion delivery design (#4147)",
    )
}

fn spawn_goal_field_too_large(field: &'static str, len: usize) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        format!(
            "spawn_subagent {field} is too large: {len} bytes (max {DEFAULT_SUBAGENT_GOAL_MAX_BYTES})"
        ),
    )
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
