// arch-exempt: large_file, host-managed model error contract remains at the crate service, plan #4088
//! Loop host adapters for IronClaw Reborn.
//!
//! This crate adapts durable Reborn support boundaries (threads/transcripts plus
//! host-managed model gateways) into the narrow `AgentLoopHost` ports. It does
//! not own provider clients, tool dispatchers, secrets, or runtime handles.
#![warn(unreachable_pub)]

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

mod await_edge_port;
mod budget_accountant;
mod budget_cost_table;
mod budget_seeding;
mod cancellation_port;
mod capability_allow_set;
mod capability_info;
mod capability_port;
mod capability_surface_filter;
mod compaction_task;
mod context_shadow;
mod context_window_cache;
mod driver_host_port_adapters;
mod external_tool_capability;
mod filesystem_skill_bundle_source;
pub mod identity_context;
mod input_port;
mod input_queue;
mod memory_context;
mod model_capability_view;
mod model_gateway;
mod model_gateway_error_mapping;
mod model_routes;
mod model_visible_scrub;
mod prompt_context_budget;
mod result_read;
mod skill_bundle_context_source;
mod skill_bundle_source;
mod skill_context;
mod subagent_prompt_port;
mod subagent_spawn_port;
mod surface_disclosure;
mod synthetic_capability;
mod system_inference;
mod thread_resolving_model_gateway;
mod thread_scope;
mod token_estimator;
mod tool_disclosure;
mod tool_disclosure_mode;
mod tool_disclosure_port;
pub mod user_profile_context;

pub use await_edge_port::{
    AwaitEdgeSettler, AwaitEdgeWriter, ResolveOutcome, ResolveReport, ScopeRecoveryInProgress,
};
pub use budget_accountant::GovernorBackedAccountant;
pub use budget_cost_table::{ModelCost, ModelCostTable, StaticModelCostTable, ZeroCostTable};
pub use budget_seeding::BudgetSeedingPolicy;
pub use cancellation_port::{
    AgentTurnRunCancellationFactory, AlwaysAliveLoopCancellationPort,
    AlwaysAliveRunCancellationFactory, CompositeTurnRunWakeNotifier, ProductLiveCancellationProbe,
    ProductLiveCancellationReadiness, RunCancellationFactory, RunCancellationHandle,
    RunCancellationObservationKind, RunStateLoopCancellationPort,
    verify_product_live_cancellation_probe,
};
pub use capability_allow_set::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
};
pub use capability_port::{
    CapabilityResultWrite, CapabilityTrajectoryObserver, CapabilityWriteResult,
    DecoratingLoopCapabilityPortFactory, DurablePersistence, HostRuntimeLoopCapabilityPort,
    HostRuntimeLoopCapabilityPortFactory, LoopCapabilityInputResolver, LoopCapabilityPortDecorator,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter, concurrency_hint_from_effects,
    loop_driver_execution_extension_id,
};
pub use capability_surface_filter::{
    CapabilitySurfaceDenyFilter, CapabilitySurfaceProfileFilter, CapabilitySurfaceVisibleFilter,
    PerSurfaceCapabilityDenyDecorator,
};
pub use compaction_task::{
    ACTIVE_TASK_COMPACTION_PROMPT_ID, DEFAULT_COMPACTION_PROMPT_ID, HostManagedLoopCompactionPort,
    active_task_compaction_prompt_id, default_compaction_prompt_id,
    default_host_managed_loop_compaction_port, host_managed_loop_compaction_port_with_prompt_id,
};
pub use context_window_cache::ThreadContextWindowCache;
pub use driver_host_port_adapters::{
    HostManagedLoopCheckpointPort, HostManagedLoopProgressPort, NoExtraLoopInputPort,
    turn_error_to_host_error,
};
pub use external_tool_capability::wrap_external_tools;
pub use filesystem_skill_bundle_source::{FilesystemSkillBundleRoot, FilesystemSkillBundleSource};
pub use identity_context::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostIdentityMessageContent, IdentityApplicability, IdentityBudget, IdentityFileName,
    IdentityMessageBuildOutcome, IdentityTrustLevel, build_identity_messages,
    build_identity_messages_for_run_detailed, identity_applicability_allowed_for_run,
    identity_message_ref,
};
pub use input_port::HostQueueLoopInputPort;
pub use input_queue::{HostInputBatch, HostInputEnvelope, HostInputQueue, HostInputQueueError};
pub use ironclaw_loop_contracts::PromptContextTokenBudget;
pub use model_gateway::{
    LlmModelProfilePolicy, LlmProviderModelGateway, ModelRouteProviderPool,
    REBORN_COLLAPSE_REPEATED_FAILURES_ENV, RoutedLlmProviderModelGateway,
    StaticModelRouteProviderPool, ThreadBackedLoopModelGateway,
};
pub use model_routes::{
    ActiveModelRouteSettings, ModelRoute, ModelRouteError, ModelRouteErrorKind, ModelRoutePolicy,
    ModelRouteProviderKey, ModelRouteResolver, ModelRouteSource, ModelSelectionMode, ModelSlot,
    ResolvedModelRouteSnapshot, StaticModelRouteResolver,
};
pub use model_visible_scrub::scrub_model_visible_detail;
pub use result_read::{RESULT_READ_CAPABILITY_ID, result_read_capability};
#[cfg(feature = "test-support")]
pub use result_read::{RESULT_READ_CAPABILITY_ID_FOR_TEST, wrap_result_read_capability_for_test};
pub use skill_bundle_context_source::SkillBundleContextSource;
pub use skill_bundle_source::{
    SkillBundleDescriptor, SkillBundleId, SkillBundleProvenance, SkillBundleSource,
    SkillBundleSourceError, SkillFilePath, SkillSourceKind, sort_skill_bundle_descriptors,
};
pub use skill_context::{
    HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextCandidatePayload,
    HostSkillContextSource, build_skill_run_snapshot,
};
pub use subagent_prompt_port::{
    DEFAULT_SUBAGENT_GOAL_MAX_BYTES, SubagentLoopPromptPort, SubagentPromptComposer,
    SubagentPromptGoal, SubagentPromptLimits, SubagentPromptMaterial, SubagentPromptMaterialSource,
    materialize_direction_message, materialize_goal_framing_message, materialize_goal_message,
    subagent_run_id_from_context,
};
pub use subagent_spawn_port::{
    AwaitedChildSetRecord, DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, DEFAULT_SUBAGENT_MAX_DEPTH,
    DEFAULT_SUBAGENT_MAX_SPAWN_PER_TURN, DEFAULT_SUBAGENT_MAX_TREE_DESCENDANTS,
    InMemoryAwaitEdgeWriter, JsonSpawnSubagentInputCodec, SpawnSubagentArgs,
    SpawnSubagentFlavorDescriptor, SpawnSubagentInputCodec, SpawnSubagentMode, SubagentDefinition,
    SubagentDefinitionResolver, SubagentGoalRecord, SubagentKindId, SubagentSpawnCapabilityPort,
    SubagentSpawnDeps, SubagentSpawnLimits, SubagentThreadKind, SubagentThreadMetadata,
    build_spawn_subagent_parameters_schema,
};
pub use surface_disclosure::wrap_surface_disclosure;
pub use synthetic_capability::{
    SyntheticCapability, SyntheticCapabilityDescriptor, SyntheticCapabilityHandler,
    SyntheticCapabilityInvocation, wrap_synthetic_capabilities,
};
pub use system_inference::{GuardedSystemInferencePort, ModelGatewayBackedSystemInferencePort};
pub use thread_resolving_model_gateway::{
    ThreadResolvingLoopModelGateway, ThreadResolvingLoopModelGatewayParts,
};
pub use thread_scope::ThreadScopeResolver;
pub use tool_disclosure::bridge_capability_ids;
pub use tool_disclosure_mode::{REBORN_TOOL_DISCLOSURE_ENV, ToolDisclosureMode};
pub use tool_disclosure_port::ToolDisclosureCapabilityDecorator;
pub use user_profile_context::{EmptyUserProfileSource, HostUserProfileSource};
pub const COMPACTION_SYSTEM_PROMPT: &str =
    include_str!("../prompts/compaction_summarizer_fresh.md");
pub const ACTIVE_TASK_COMPACTION_SYSTEM_PROMPT: &str = concat!(
    include_str!("../prompts/compaction_summarizer_fresh.md"),
    "\n\n",
    include_str!("../prompts/active_task_compaction_append.md"),
);
pub use token_estimator::{
    CHARS_PER_TOKEN_DEFAULT, EstimatedTokenCount, estimate_tokens_from_chars,
};

use tokio::sync::{Mutex, OnceCell};

use async_trait::async_trait;
use ironclaw_host_api::ids::RunId;
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, AgentLoopHostErrorReasonKind,
    AppendCapabilityResultRef, AssistantReply, BeginAssistantDraft, CapabilityDeniedReasonKind,
    CapabilitySurfaceVersion, FinalizeAssistantMessage, InstructionMaterializationStore,
    LoopCapabilityPort, LoopContextBundle, LoopContextCompactionKind,
    LoopContextCompactionMetadata, LoopContextMessage, LoopContextPort, LoopContextRequest,
    LoopContextSnippet, LoopDriverNoteKind, LoopHostMilestoneEmitter, LoopHostMilestoneSink,
    LoopInputCursor, LoopModelMessage, LoopModelPort, LoopModelRequest, LoopModelResponse,
    LoopModelUsage, LoopPromptBundleAuthority, LoopRequest, LoopRequestBatch, LoopRunContext,
    LoopRunInfoPort, LoopSafeSummary, LoopTranscriptPort, MemoryPromptContextService,
    ModelProfileId, ModelStreamChunk, ParentLoopOutput, PromptMode, UpdateAssistantDraft,
    VisibleCapabilityRequest, VisibleCapabilitySurface, resolution, sanitize_model_visible_text,
    sort_instruction_snippets_for_prompt,
};
use ironclaw_outbound::{
    OutboundError, ReplyAttachmentHandle, ReplyAttachmentIntent, ReplyAttachmentIntentPort,
};
use ironclaw_threads::{
    AppendAssistantDraftRequest, AppendFinalizedAssistantMessageRequest,
    AppendToolResultReferenceRequest, AttachmentKind, AttachmentRef, ContextMessage,
    FinalizedAssistantMessageByRunRequest, LoadContextMessagesRequest, LoadContextWindowRequest,
    MessageContent, MessageKind, MessageStatus, ProviderToolCallReferenceEnvelope,
    SessionThreadError, SessionThreadService, SummaryArtifact, ThreadHistoryRequest,
    ThreadMessageId, ThreadMessageRecord, ThreadScope, ToolResultReferenceEnvelope,
    ToolResultSafeSummary, UpdateAssistantDraftRequest,
};
use ironclaw_turns::{LoopGateRef, LoopMessageRef, TurnId, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

const EMPTY_SURFACE_VERSION: &str = "empty:v1";
const LOOP_SYSTEM_ROLE: &str = "system";

fn trace_loop_host_latency_ok(
    operation: &'static str,
    context: &LoopRunContext,
    started_at: Option<Instant>,
    max_messages: usize,
    message_count: usize,
) {
    // Keep the pre-rename component label stable for dashboard continuity.
    ironclaw_observability::live_latency_trace_ok!(
        "loop_support",
        operation,
        started_at,
        tenant_id = %context.scope.tenant_id,
        agent_id = context.scope.agent_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        project_id = context.scope.project_id.as_ref().map(|id| id.as_str()).unwrap_or(""),
        thread_id = %context.thread_id,
        owner_user_id = context.scope.explicit_owner_user_id().map(|id| id.as_str()).unwrap_or(""),
        run_id = %context.run_id,
        turn_id = %context.turn_id,
        max_messages = max_messages as u64,
        message_count = message_count as u64,
        "loop support operation completed",
    );
}

pub fn raw_agent_loop_host_error(
    component: &'static str,
    operation: &'static str,
    kind: AgentLoopHostErrorKind,
    safe_summary: impl Into<String>,
    raw_detail: impl std::fmt::Display,
) -> AgentLoopHostError {
    let safe_summary = safe_summary.into();
    let raw_detail = raw_detail.to_string();
    tracing::debug!(
        component,
        operation,
        kind = ?kind,
        safe_summary = %safe_summary,
        raw_detail = %raw_detail,
        "agent loop host error mapped to safe summary"
    );
    // Carry the raw cause to the model as a secret-scrubbed diagnostic. Only
    // secret VALUES are redacted (via the full leak-detector registry + prefix
    // matcher) and any injection payload is fenced; paths/codes/raw error text
    // reach the model so it can retry or explain. The word/delimiter ban is NOT
    // applied here.
    let mut error = AgentLoopHostError::new(kind, safe_summary);
    let scrubbed = scrub_model_visible_detail(raw_detail);
    if !scrubbed.trim().is_empty() {
        error = error.with_detail(scrubbed);
    }
    error
}

pub fn raw_host_managed_model_error(
    component: &'static str,
    operation: &'static str,
    kind: HostManagedModelErrorKind,
    safe_summary: impl Into<String>,
    raw_detail: impl std::fmt::Display,
) -> HostManagedModelError {
    let safe_summary = safe_summary.into();
    tracing::warn!(
        component,
        operation,
        kind = ?kind,
        safe_summary = %safe_summary,
        raw_detail = %raw_detail,
        "host-managed model error mapped to safe summary"
    );
    HostManagedModelError::safe(kind, safe_summary)
}

/// Thread-backed context adapter for text-only Reborn loops.
#[derive(Clone)]
pub struct ThreadBackedLoopContextPort<S>
where
    S: SessionThreadService + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    run_context: LoopRunContext,
    max_messages: usize,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    identity_budget: IdentityBudget,
    prompt_context_budget: PromptContextTokenBudget,
    context_window_cache: Option<Arc<ThreadContextWindowCache>>,
    identity_candidates: Arc<IdentityCandidateCache>,
    milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
    /// Optional proactive-memory source. When wired, memory snippets are fetched
    /// ONCE per run (cached in `memory_snippets_cache`) and surfaced into the
    /// prompt's `"memory"` section; when absent, `memory_snippets` stays empty.
    /// Optional; production wires `None` pending #5013 — a composition without a
    /// memory backend degrades to no memory, never failing the turn. (Unlike the
    /// non-optional null-object `user_profile_source`, this is a genuine `Option`.)
    // arch-exempt: optional_arc, deferred production wiring, issue #5013
    memory_context_service: Option<Arc<dyn MemoryPromptContextService>>,
    /// Per-run cache for the fetched memory snippets. Shared across clones via
    /// `Arc` so the "fetch once per run" guarantee holds even if the port is
    /// cloned, exactly like `identity_candidates`.
    memory_snippets_cache: Arc<OnceCell<Vec<LoopContextSnippet>>>,
}

struct IdentityCandidateCache {
    text_only: OnceCell<Vec<HostIdentityContextCandidate>>,
    codeact: OnceCell<Vec<HostIdentityContextCandidate>>,
    text_only_personal_context_admitted: OnceCell<()>,
    codeact_personal_context_admitted: OnceCell<()>,
    text_only_personal_context_admitted_in_flight: AtomicBool,
    codeact_personal_context_admitted_in_flight: AtomicBool,
}

impl IdentityCandidateCache {
    fn new() -> Self {
        Self {
            text_only: OnceCell::new(),
            codeact: OnceCell::new(),
            text_only_personal_context_admitted: OnceCell::new(),
            codeact_personal_context_admitted: OnceCell::new(),
            text_only_personal_context_admitted_in_flight: AtomicBool::new(false),
            codeact_personal_context_admitted_in_flight: AtomicBool::new(false),
        }
    }

    fn cell_for_mode(&self, mode: PromptMode) -> &OnceCell<Vec<HostIdentityContextCandidate>> {
        match mode {
            PromptMode::TextOnly => &self.text_only,
            PromptMode::CodeAct => &self.codeact,
        }
    }

    fn personal_context_admitted_cell_for_mode(&self, mode: PromptMode) -> &OnceCell<()> {
        match mode {
            PromptMode::TextOnly => &self.text_only_personal_context_admitted,
            PromptMode::CodeAct => &self.codeact_personal_context_admitted,
        }
    }

    fn personal_context_admitted_in_flight_for_mode(&self, mode: PromptMode) -> &AtomicBool {
        match mode {
            PromptMode::TextOnly => &self.text_only_personal_context_admitted_in_flight,
            PromptMode::CodeAct => &self.codeact_personal_context_admitted_in_flight,
        }
    }
}

impl<S> ThreadBackedLoopContextPort<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        run_context: LoopRunContext,
        max_messages: usize,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            run_context,
            max_messages,
            skill_context_source: None,
            identity_context_source: None,
            identity_budget: IdentityBudget::default(),
            prompt_context_budget: PromptContextTokenBudget::default(),
            context_window_cache: None,
            identity_candidates: Arc::new(IdentityCandidateCache::new()),
            milestone_sink: None,
            memory_context_service: None,
            memory_snippets_cache: Arc::new(OnceCell::new()),
        }
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
        self
    }

    /// Installs the proactive-memory source. When wired, the loop fetches both
    /// the short-term (per-thread) and long-term memory lanes ONCE at the first
    /// prompt build of the run, caches the admitted snippets, and surfaces them
    /// into the prompt every turn. When not called the loop carries no memory.
    pub fn with_memory_context_service(
        mut self,
        service: Arc<dyn MemoryPromptContextService>,
    ) -> Self {
        self.memory_context_service = Some(service);
        self
    }

    pub fn with_identity_context_source(
        mut self,
        source: Arc<dyn HostIdentityContextSource>,
    ) -> Self {
        self.identity_context_source = Some(source);
        self
    }

    pub fn with_identity_budget(mut self, budget: IdentityBudget) -> Self {
        self.identity_budget = budget;
        self
    }

    pub fn with_prompt_context_token_budget(mut self, budget: PromptContextTokenBudget) -> Self {
        self.prompt_context_budget = budget;
        self
    }

    pub fn with_context_window_cache(mut self, cache: Arc<ThreadContextWindowCache>) -> Self {
        self.context_window_cache = Some(cache);
        self
    }

    pub fn with_milestone_sink(mut self, sink: Arc<dyn LoopHostMilestoneSink>) -> Self {
        self.milestone_sink = Some(sink);
        self
    }
}

impl<S> LoopRunInfoPort for ThreadBackedLoopContextPort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl<S> LoopContextPort for ThreadBackedLoopContextPort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        validate_thread_scope_for_run(&self.thread_scope, &self.run_context)?;
        validate_context_cursor(request.after.as_ref(), &self.run_context)?;
        let max_messages = bounded_limit(request.limit, self.max_messages);
        let mode = request.mode;
        let context_window = async {
            let started_at = ironclaw_observability::live_latency_started_at();
            let context = self
                .thread_service
                .load_context_window(LoadContextWindowRequest {
                    scope: self.thread_scope.clone(),
                    thread_id: self.run_context.thread_id.clone(),
                    max_messages,
                })
                .await
                .map_err(context_read_error)?;
            trace_loop_host_latency_ok(
                "context_load_window",
                &self.run_context,
                started_at,
                max_messages,
                context.messages.len(),
            );
            if let Some(cache) = self.context_window_cache.as_ref() {
                let started_at = ironclaw_observability::live_latency_started_at();
                cache
                    .store(self.thread_scope.clone(), max_messages, context.clone())
                    .await;
                trace_loop_host_latency_ok(
                    "context_cache_store",
                    &self.run_context,
                    started_at,
                    max_messages,
                    context.messages.len(),
                );
            }
            Ok::<_, AgentLoopHostError>(context)
        };

        let skill_snippets = async {
            let started_at = ironclaw_observability::live_latency_started_at();
            let instruction_snippets = match self.skill_context_source.as_deref() {
                Some(source) => {
                    skill_context::build_skill_instruction_snippets(source, &self.run_context)
                        .await?
                }
                None => Vec::new(),
            };
            trace_loop_host_latency_ok(
                "context_skill_snippets",
                &self.run_context,
                started_at,
                max_messages,
                instruction_snippets.len(),
            );
            Ok::<_, AgentLoopHostError>(instruction_snippets)
        };

        let identity_context = async {
            let started_at = ironclaw_observability::live_latency_started_at();
            let (identity_messages, admitted_personal_context_paths) =
                match self.identity_context_source.as_deref() {
                    Some(source) => {
                        let candidates = self
                            .identity_candidates
                            .cell_for_mode(mode)
                            .get_or_try_init(|| async {
                                source
                                    .load_identity_candidates(&self.run_context, mode)
                                    .await
                                    .map_err(HostIdentityContextBuildError::into_host_error)
                            })
                            .await?;
                        let outcome = identity_context::build_identity_messages_for_run_detailed(
                            candidates,
                            &self.run_context,
                            mode,
                            self.identity_budget,
                        )?;
                        (outcome.messages, outcome.admitted_personal_context_paths)
                    }
                    None => (Vec::new(), Vec::new()),
                };
            trace_loop_host_latency_ok(
                "context_identity_messages",
                &self.run_context,
                started_at,
                max_messages,
                identity_messages.len(),
            );
            Ok::<_, AgentLoopHostError>((identity_messages, admitted_personal_context_paths))
        };

        let (context, instruction_snippets, (identity_messages, admitted_personal_context_paths)) =
            tokio::try_join!(context_window, skill_snippets, identity_context)?;
        self.publish_personal_context_admitted(mode, &admitted_personal_context_paths);

        // Proactive memory: fetch both lanes ONCE per run (cached) using the
        // latest user message as the query, and surface them into the prompt's
        // "memory" section. Derived from `context.messages` before the move below.
        let memory_snippets = self.load_memory_snippets_once(&context.messages).await;

        let started_at = ironclaw_observability::live_latency_started_at();
        let compaction_message_index = context
            .messages
            .iter()
            .filter_map(context_message_to_compaction_metadata)
            .collect();
        let messages = prompt_context_budget::select_prompt_context_messages(
            context.messages,
            self.prompt_context_budget,
        );
        trace_loop_host_latency_ok(
            "context_select_messages",
            &self.run_context,
            started_at,
            max_messages,
            messages.len(),
        );

        Ok(LoopContextBundle {
            identity_messages,
            messages: messages
                .into_iter()
                .filter_map(context_message_to_loop_message)
                .collect(),
            compaction_message_index,
            instruction_snippets,
            memory_snippets,
        })
    }
}

impl<S> ThreadBackedLoopContextPort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    fn publish_personal_context_admitted(
        &self,
        mode: PromptMode,
        admitted_paths: &[IdentityFileName],
    ) {
        if admitted_paths.is_empty() {
            return;
        }
        let Some(milestone_sink) = self.milestone_sink.as_ref() else {
            return;
        };
        let emitted_cell = self
            .identity_candidates
            .personal_context_admitted_cell_for_mode(mode);
        if emitted_cell.get().is_some() {
            return;
        }
        let in_flight = self
            .identity_candidates
            .personal_context_admitted_in_flight_for_mode(mode);
        if in_flight.swap(true, Ordering::AcqRel) {
            return;
        }
        let summary = match personal_context_admitted_summary(admitted_paths) {
            Ok(summary) => summary,
            Err(error) => {
                in_flight.store(false, Ordering::Release);
                tracing::debug!("failed to build personal context admitted milestone: {error}");
                return;
            }
        };
        let context = self.run_context.clone();
        let milestone_sink = Arc::clone(milestone_sink);
        let identity_candidates = Arc::clone(&self.identity_candidates);
        tokio::spawn(async move {
            let publish_result = LoopHostMilestoneEmitter::new(context, milestone_sink)
                .driver_note(LoopDriverNoteKind::Context, summary)
                .await;
            if let Err(error) = publish_result {
                tracing::debug!("failed to emit personal context admitted milestone: {error}");
            } else {
                let _ = identity_candidates
                    .personal_context_admitted_cell_for_mode(mode)
                    .set(());
            }
            identity_candidates
                .personal_context_admitted_in_flight_for_mode(mode)
                .store(false, Ordering::Release);
        });
    }
}

fn personal_context_admitted_summary(
    admitted_paths: &[IdentityFileName],
) -> Result<LoopSafeSummary, AgentLoopHostError> {
    let source_labels = admitted_paths
        .iter()
        .filter_map(|path| personal_context_source_label(path.as_str()))
        .collect::<Vec<_>>()
        .join(" ");
    let summary = if source_labels.is_empty() {
        format!("personal context admitted count {}", admitted_paths.len())
    } else {
        format!(
            "personal context admitted count {} sources {}",
            admitted_paths.len(),
            source_labels
        )
    };
    LoopSafeSummary::new(summary).map_err(|reason| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("personal context milestone summary invalid: {reason}"),
        )
    })
}

fn personal_context_source_label(path: &str) -> Option<String> {
    let basename = path
        .rsplit(['/', '\\'])
        .next()
        .filter(|label| !label.is_empty())
        .unwrap_or(path);
    let label = basename
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
        .collect::<String>();
    (!label.is_empty()).then_some(label)
}

/// Thread-backed transcript adapter for text-only assistant replies.
#[derive(Clone)]
pub struct ThreadBackedLoopTranscriptPort<S>
where
    S: SessionThreadService + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    run_context: LoopRunContext,
    milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
    reply_attachment_intent_port: Option<Arc<dyn ReplyAttachmentIntentPort>>,
    // Only successful milestone publications are recorded here: if best-effort
    // publishing fails after the transcript write, an idempotent retry can try again.
    emitted_assistant_reply_finalized_refs: Arc<Mutex<HashSet<String>>>,
}

const TRANSCRIPT_WRITE_MAX_ATTEMPTS: usize = 3;
const TRANSCRIPT_WRITE_RETRY_BASE_DELAY_MS: u64 = 10;

impl<S> ThreadBackedLoopTranscriptPort<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        run_context: LoopRunContext,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            run_context,
            milestone_sink: None,
            reply_attachment_intent_port: None,
            emitted_assistant_reply_finalized_refs: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn with_milestone_sink(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        run_context: LoopRunContext,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            run_context,
            milestone_sink: Some(milestone_sink),
            reply_attachment_intent_port: None,
            emitted_assistant_reply_finalized_refs: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn with_reply_attachment_intent_port(
        mut self,
        port: Arc<dyn ReplyAttachmentIntentPort>,
    ) -> Self {
        self.reply_attachment_intent_port = Some(port);
        self
    }
}

impl<S> LoopRunInfoPort for ThreadBackedLoopTranscriptPort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl<S> LoopTranscriptPort for ThreadBackedLoopTranscriptPort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    async fn begin_assistant_draft(
        &self,
        request: BeginAssistantDraft,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        validate_thread_scope_for_run(&self.thread_scope, &self.run_context)?;
        let draft = self
            .thread_service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                turn_run_id: self.run_context.run_id.to_string(),
                content: MessageContent::text(request.reply.content),
            })
            .await
            .map_err(transcript_write_error)?;
        message_ref(draft.message_id)
    }

    async fn update_assistant_draft(
        &self,
        request: UpdateAssistantDraft,
    ) -> Result<(), AgentLoopHostError> {
        validate_thread_scope_for_run(&self.thread_scope, &self.run_context)?;
        let message_id = message_id_from_ref(&request.message_ref)?;
        self.load_current_run_message(message_id).await?;
        self.thread_service
            .update_assistant_draft(UpdateAssistantDraftRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                message_id,
                content: MessageContent::text(request.reply.content),
            })
            .await
            .map_err(transcript_write_error)?;
        Ok(())
    }

    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        validate_thread_scope_for_run(&self.thread_scope, &self.run_context)?;
        let reply_content = self.finalized_reply_content(request.reply.content).await?;
        let turn_run_id = self.run_context.run_id.to_string();
        let append_request = AppendFinalizedAssistantMessageRequest {
            scope: self.thread_scope.clone(),
            thread_id: self.run_context.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
            content: reply_content.clone(),
        };
        let finalized = match retry_transcript_backend_write(
            &turn_run_id,
            "append_finalized_assistant_message",
            || {
                self.thread_service
                    .append_finalized_assistant_message(append_request.clone())
            },
        )
        .await
        {
            Ok(message) => message,
            Err(error) => {
                if let Some(message) = self
                    .already_finalized_matching_reply_for_current_run(&reply_content)
                    .await?
                {
                    message
                } else {
                    return Err(transcript_write_error(error));
                }
            }
        };
        if finalized.status != MessageStatus::Finalized
            || persisted_message_content(&finalized).as_ref() != Some(&reply_content)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::TranscriptWriteFailed,
                "assistant transcript write failed",
            ));
        }
        let message_ref = message_ref(finalized.message_id)?;
        self.emit_assistant_reply_finalized(message_ref.clone())
            .await?;
        Ok(message_ref)
    }

    async fn append_capability_result_ref(
        &self,
        request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        validate_thread_scope_for_run(&self.thread_scope, &self.run_context)?;
        // Fail soft on a summary that trips either strict validator: the
        // summary is only the inline label for the result reference (the model
        // sees the real output via the result ref / observation), so a
        // malformed label must not end the run
        // (capability-access contract). Degrade to a
        // fixed host-authored marker; the raw text stays out of the transcript.
        let safe_summary = LoopSafeSummary::new(request.safe_summary)
            .and_then(|summary| {
                ToolResultSafeSummary::new(summary.as_str().to_string())
                    .map_err(|error| error.to_string())
            })
            .unwrap_or_else(|reason| {
                tracing::debug!(
                    %reason,
                    "tool result summary failed strict validation; degrading to redaction marker"
                );
                ToolResultSafeSummary::redacted_tool_result_summary()
            });
        let model_observation = request
            .model_observation
            .and_then(|observation| match observation.validate() {
                Ok(()) => match serde_json::to_value(observation) {
                    Ok(value) => Some(value),
                    Err(error) => {
                        tracing::warn!(
                            reason = %error,
                            "dropping unserializable model-visible tool observation and preserving safe summary"
                        );
                        None
                    }
                },
                Err(error) => {
                    tracing::warn!(
                        reason = %error,
                        "dropping invalid model-visible tool observation and preserving safe summary"
                    );
                    None
                }
            });
        let turn_run_id = self.run_context.run_id.to_string();
        let append_request = AppendToolResultReferenceRequest {
            scope: self.thread_scope.clone(),
            thread_id: self.run_context.thread_id.clone(),
            turn_run_id: turn_run_id.clone(),
            result_ref: request.result_ref.as_str().to_string(),
            safe_summary,
            model_observation,
            provider_call: request
                .provider_call
                .map(provider_call_reference_to_envelope),
        };
        let record =
            retry_transcript_backend_write(&turn_run_id, "append_tool_result_reference", || {
                self.thread_service
                    .append_tool_result_reference(append_request.clone())
            })
            .await
            .map_err(transcript_write_error)?;
        message_ref(record.message_id)
    }
}

async fn retry_transcript_backend_write<T, F, Fut>(
    turn_run_id: &str,
    operation: &'static str,
    mut write: F,
) -> Result<T, SessionThreadError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, SessionThreadError>>,
{
    let mut attempt = 1;
    loop {
        match write().await {
            Err(SessionThreadError::Backend(_)) if attempt < TRANSCRIPT_WRITE_MAX_ATTEMPTS => {
                tracing::warn!(
                    operation,
                    attempt,
                    max_attempts = TRANSCRIPT_WRITE_MAX_ATTEMPTS,
                    "transcript backend write failed; retrying exact idempotent write"
                );
                tokio::time::sleep(transcript_write_retry_delay(turn_run_id, attempt)).await;
                attempt += 1;
            }
            result => return result,
        }
    }
}

fn transcript_write_retry_delay(turn_run_id: &str, failed_attempt: usize) -> Duration {
    let exponent = u32::try_from(failed_attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    let base_delay_ms = TRANSCRIPT_WRITE_RETRY_BASE_DELAY_MS
        .checked_shl(exponent)
        .unwrap_or(u64::MAX);
    let jitter_seed = turn_run_id
        .bytes()
        .fold(failed_attempt as u64, |seed, byte| {
            seed.wrapping_mul(31).wrapping_add(u64::from(byte))
        });
    Duration::from_millis(base_delay_ms.saturating_add(jitter_seed % base_delay_ms))
}

impl<S> ThreadBackedLoopTranscriptPort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    async fn emit_assistant_reply_finalized(
        &self,
        message_ref: LoopMessageRef,
    ) -> Result<(), AgentLoopHostError> {
        let Some(milestone_sink) = &self.milestone_sink else {
            return Ok(());
        };

        let mut emitted_refs = self.emitted_assistant_reply_finalized_refs.lock().await;
        if emitted_refs.contains(message_ref.as_str()) {
            return Ok(());
        }

        let milestones =
            LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(milestone_sink));
        if let Err(error) = milestones
            .assistant_reply_finalized(message_ref.clone())
            .await
        {
            tracing::debug!(
                kind = ?error.kind,
                "loop assistant_reply_finalized milestone failed after finalized transcript write"
            );
            return Ok(());
        }
        emitted_refs.insert(message_ref.as_str().to_string());
        Ok(())
    }

    async fn load_current_run_message(
        &self,
        message_id: ThreadMessageId,
    ) -> Result<ThreadMessageRecord, AgentLoopHostError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
            })
            .await
            .map_err(transcript_write_error)?;
        let message = history
            .messages
            .into_iter()
            .find(|message| message.message_id == message_id)
            .ok_or_else(invalid_transcript_ref_error)?;
        let expected_run_id = self.run_context.run_id.to_string();
        if message.turn_run_id.as_deref() != Some(expected_run_id.as_str()) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "transcript message does not belong to this loop run",
            ));
        }
        Ok(message)
    }

    async fn already_finalized_matching_reply_for_current_run(
        &self,
        reply_content: &MessageContent,
    ) -> Result<Option<ThreadMessageRecord>, AgentLoopHostError> {
        let Some(message) = self
            .thread_service
            .finalized_assistant_message_by_run(FinalizedAssistantMessageByRunRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                turn_run_id: self.run_context.run_id.to_string(),
            })
            .await
            .map_err(transcript_write_error)?
        else {
            return Ok(None);
        };
        if persisted_message_content(&message).as_ref() == Some(reply_content) {
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }

    async fn finalized_reply_content(
        &self,
        reply_text: String,
    ) -> Result<MessageContent, AgentLoopHostError> {
        let Some(port) = self.reply_attachment_intent_port.as_ref() else {
            return Ok(MessageContent::text(reply_text));
        };
        let mut scope = self.thread_scope.to_resource_scope();
        scope.thread_id = Some(self.run_context.thread_id.clone());
        let run_id = RunId::from_uuid(self.run_context.run_id.as_uuid());
        let intents = port
            .seal(&scope, &run_id)
            .await
            .map_err(reply_attachment_seal_error)?;
        let attachments = reply_attachment_refs(&run_id, intents);
        let reply_text =
            ironclaw_threads::deproject_model_attachment_context(reply_text, &attachments);
        Ok(MessageContent::with_attachments(reply_text, attachments))
    }
}

fn reply_attachment_refs(
    run_id: &RunId,
    intents: Vec<ReplyAttachmentIntent>,
) -> Vec<AttachmentRef> {
    intents
        .into_iter()
        .map(|intent| AttachmentRef {
            id: ReplyAttachmentHandle::for_run_path(run_id, &intent.path).to_string(),
            kind: AttachmentKind::from_mime_type(&intent.mime_type),
            mime_type: intent.mime_type,
            filename: Some(intent.filename),
            size_bytes: Some(intent.size_bytes),
            storage_key: Some(intent.path.to_string()),
            extracted_text: None,
        })
        .collect()
}

fn persisted_message_content(message: &ThreadMessageRecord) -> Option<MessageContent> {
    message.content.as_ref().map(|content| {
        MessageContent::with_attachments(content.clone(), message.attachments.clone())
    })
}

fn reply_attachment_seal_error(error: OutboundError) -> AgentLoopHostError {
    tracing::debug!(error = %error, "reply attachment finalization failed");
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::TranscriptWriteFailed,
        "reply attachment finalization failed",
    )
}

/// Empty capability surface for the text-only loop-host MVP.
#[derive(Debug, Clone, Default)]
pub struct EmptyLoopCapabilityPort;

#[async_trait]
impl ironclaw_loop_contracts::LoopCapabilityPort for EmptyLoopCapabilityPort {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            version: empty_surface_version()?,
            descriptors: Vec::new(),
            callable_capability_ids: None,
        })
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<ironclaw_host_api::resolution::Resolution, AgentLoopHostError> {
        let empty_surface_version = empty_surface_version()?;
        if request.surface_version != empty_surface_version {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface is stale or unknown",
            ));
        }
        Err(empty_capability_error())
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ironclaw_host_api::resolution::ResolutionBatch, AgentLoopHostError> {
        let empty_surface_version = empty_surface_version()?;
        if request
            .invocations
            .iter()
            .any(|invocation| invocation.surface_version != empty_surface_version)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::StaleSurface,
                "capability surface is stale or unknown",
            ));
        }
        let resolutions = request
            .invocations
            .into_iter()
            .map(|_| {
                resolution::denied(
                    CapabilityDeniedReasonKind::EmptySurface,
                    "no capabilities are available to this loop".to_string(),
                )
                .resolution
            })
            .collect();
        Ok(ironclaw_host_api::resolution::ResolutionBatch {
            resolutions,
            stopped_on_suspension: false,
        })
    }
}

/// Thread-backed model adapter that resolves loop message references before
/// delegating completion to a host-managed gateway.
#[derive(Clone)]
pub struct ThreadBackedLoopModelPort<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    thread_service: Arc<S>,
    thread_scope: ThreadScope,
    run_context: LoopRunContext,
    gateway: Arc<G>,
    capabilities: Option<Arc<dyn LoopCapabilityPort>>,
    max_messages: usize,
    prompt_context_budget: PromptContextTokenBudget,
    context_window_cache: Option<Arc<ThreadContextWindowCache>>,
    prompt_authority: LoopPromptBundleAuthority,
    milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    attachment_read_port: Option<Arc<dyn LoopAttachmentReadPort>>,
    stream_sink: Option<Arc<dyn HostManagedModelStreamSink>>,
}

impl<S, G> ThreadBackedLoopModelPort<S, G>
where
    S: SessionThreadService + ?Sized,
    G: HostManagedModelGateway + ?Sized,
{
    pub fn new(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        run_context: LoopRunContext,
        gateway: Arc<G>,
        max_messages: usize,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            run_context,
            gateway,
            capabilities: None,
            max_messages,
            prompt_context_budget: PromptContextTokenBudget::default(),
            context_window_cache: None,
            prompt_authority: LoopPromptBundleAuthority::shared(),
            milestone_sink: None,
            skill_context_source: None,
            instruction_materialization_store: None,
            identity_context_source: None,
            attachment_read_port: None,
            stream_sink: None,
        }
    }

    pub fn with_milestone_sink(
        thread_service: Arc<S>,
        thread_scope: ThreadScope,
        run_context: LoopRunContext,
        gateway: Arc<G>,
        max_messages: usize,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            thread_service,
            thread_scope,
            run_context,
            gateway,
            capabilities: None,
            max_messages,
            prompt_context_budget: PromptContextTokenBudget::default(),
            context_window_cache: None,
            prompt_authority: LoopPromptBundleAuthority::shared(),
            milestone_sink: Some(milestone_sink),
            skill_context_source: None,
            instruction_materialization_store: None,
            identity_context_source: None,
            attachment_read_port: None,
            stream_sink: None,
        }
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
        self
    }

    pub fn with_prompt_bundle_authority(
        mut self,
        prompt_authority: LoopPromptBundleAuthority,
    ) -> Self {
        self.prompt_authority = prompt_authority;
        self
    }

    pub fn with_prompt_context_token_budget(mut self, budget: PromptContextTokenBudget) -> Self {
        self.prompt_context_budget = budget;
        self
    }

    pub fn with_context_window_cache(mut self, cache: Arc<ThreadContextWindowCache>) -> Self {
        self.context_window_cache = Some(cache);
        self
    }

    pub fn with_instruction_materialization_store(
        mut self,
        store: Arc<dyn InstructionMaterializationStore>,
    ) -> Self {
        self.instruction_materialization_store = Some(store);
        self
    }

    pub fn with_identity_context_source(
        mut self,
        source: Arc<dyn HostIdentityContextSource>,
    ) -> Self {
        self.identity_context_source = Some(source);
        self
    }

    pub fn with_capability_port(mut self, capabilities: Arc<dyn LoopCapabilityPort>) -> Self {
        self.capabilities = Some(capabilities);
        self
    }

    pub fn with_attachment_read_port(mut self, port: Arc<dyn LoopAttachmentReadPort>) -> Self {
        self.attachment_read_port = Some(port);
        self
    }

    pub fn with_stream_sink(mut self, sink: Arc<dyn HostManagedModelStreamSink>) -> Self {
        self.stream_sink = Some(sink);
        self
    }

    /// Read the raw bytes of a model-visible message's image attachments so the
    /// gateway can attach them as multimodal parts for a vision model. Returns
    /// the bytes only — base64/`data:` URL formatting is a provider concern that
    /// lives in the gateway, so this neutral adapter stays format-agnostic.
    /// Empty when no read port is wired or the message has no images.
    ///
    /// The read is deliberately *not* gated on model vision capability here. The
    /// authoritative model identity is `model_override`, resolved inside the
    /// gateway from its routing policy, which can diverge from the run-context
    /// route snapshot this port holds. Gating the read on the snapshot would
    /// risk silently dropping images whenever the two disagree, so the single
    /// authoritative vision gate lives in the gateway's `convert_messages`: a
    /// text-only model simply discards these parts and keeps the `<attachments>`
    /// text pointer. The only cost is a bounded read for the rare text-only +
    /// image case.
    ///
    /// Read failures are logged and skipped — the image is dropped rather than
    /// failing the turn; the textual `<attachments>` pointer remains either way.
    async fn read_image_parts(
        &self,
        attachments: &[ironclaw_threads::ContextImageAttachment],
    ) -> Vec<HostManagedModelImagePart> {
        if attachments.is_empty() {
            return Vec::new();
        }
        let Some(port) = self.attachment_read_port.as_ref() else {
            return Vec::new();
        };
        let scope = self.thread_scope.to_resource_scope();
        let mut parts = Vec::with_capacity(attachments.len());
        for attachment in attachments {
            match port
                .read_attachment_bytes(&scope, &attachment.storage_key)
                .await
            {
                Ok(bytes) => {
                    parts.push(HostManagedModelImagePart {
                        mime_type: attachment.mime_type.clone(),
                        bytes,
                    });
                }
                // silent-ok: an unreadable attachment is dropped, not fatal — the
                // model still gets the text and the `<attachments>` pointer; the
                // cause is logged here for diagnosis.
                Err(error) => {
                    tracing::debug!(
                        storage_key = %attachment.storage_key,
                        %error,
                        "skipping image attachment that could not be read for the model"
                    );
                }
            }
        }
        parts
    }
}

impl<S, G> LoopRunInfoPort for ThreadBackedLoopModelPort<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl<S, G> LoopModelPort for ThreadBackedLoopModelPort<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        validate_thread_scope_for_run(&self.thread_scope, &self.run_context)?;
        let requested_model_profile_id = request.model_preference.clone();
        let model_profile_id = requested_model_profile_id.clone().unwrap_or_else(|| {
            self.run_context
                .resolved_run_profile
                .model_profile_id
                .clone()
        });
        let prompt_grant = self.prompt_authority.authorize_latest_model_request(
            &self.run_context,
            &request.messages,
            &request.surface_version,
        )?;

        // Resolve messages *before* the budget reservation in the outer
        // `HostManagedLoopModelPort` so a message-resolution failure here
        // cannot orphan a reservation taken by the outer port. The inner
        // port itself never holds a reservation — budget accounting lives
        // exclusively in the outer port (see #3841 follow-up "delete dead
        // with_budget_accountant").
        let resolved_messages = self.resolve_model_messages(prompt_grant.messages).await?;

        self.emit_model_started(requested_model_profile_id).await;
        let host_request = HostManagedModelRequest {
            model_profile_id: model_profile_id.clone(),
            fallback_index: request.fallback_index,
            messages: resolved_messages,
            surface_version: request.surface_version.clone(),
            resolved_model_route: self.run_context.resolved_model_route.clone(),
            run_id: self.run_context.run_id,
            turn_id: self.run_context.turn_id,
        };
        let gateway_result = if let Some(capabilities) = self.capabilities.as_ref() {
            let capabilities: Arc<dyn LoopCapabilityPort> =
                if let Some(ref capability_view) = request.capability_view {
                    Arc::new(CapabilitySurfaceVisibleFilter::new(
                        Arc::clone(capabilities),
                        capability_view.visible_capability_ids.clone(),
                    ))
                } else {
                    Arc::clone(capabilities)
                };
            if let Some(stream_sink) = self.stream_sink.as_ref() {
                self.gateway
                    .stream_model_with_capabilities_and_progress(
                        host_request,
                        capabilities,
                        Arc::clone(stream_sink),
                    )
                    .await
            } else {
                self.gateway
                    .stream_model_with_capabilities(host_request, capabilities)
                    .await
            }
        } else if let Some(stream_sink) = self.stream_sink.as_ref() {
            self.gateway
                .stream_model_with_progress(host_request, Arc::clone(stream_sink))
                .await
        } else {
            self.gateway.stream_model(host_request).await
        };

        let host_response_result = match gateway_result {
            Ok(response) => {
                let HostManagedModelResponse {
                    safe_text_deltas,
                    safe_reasoning_deltas,
                    output,
                    usage,
                    effective_fallback_index,
                } = response;
                if effective_fallback_index != Some(request.fallback_index) {
                    Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "model gateway returned mismatched fallback route evidence",
                    ))
                } else {
                    let chunks = safe_text_deltas
                        .into_iter()
                        .map(|safe_text_delta| ModelStreamChunk {
                            safe_text_delta: sanitize_model_visible_text(safe_text_delta),
                        })
                        .collect::<Vec<_>>();
                    let loop_response = LoopModelResponse {
                        chunks,
                        safe_reasoning_deltas,
                        output,
                        effective_model_profile_id: model_profile_id.clone(),
                        usage,
                    };
                    Ok(loop_response)
                }
            }
            Err(error) => Err(model_gateway_error(error)),
        };

        match host_response_result {
            Ok(response) => {
                self.emit_model_completed(model_profile_id).await;
                Ok(response)
            }
            Err(host_error) => {
                self.emit_model_failed(host_error.kind).await;
                Err(host_error)
            }
        }
    }
}

impl<S, G> ThreadBackedLoopModelPort<S, G>
where
    S: SessionThreadService + ?Sized + Send + Sync,
    G: HostManagedModelGateway + ?Sized + Send + Sync,
{
    async fn emit_model_started(&self, requested_model_profile_id: Option<ModelProfileId>) {
        if let Some(milestone_sink) = &self.milestone_sink {
            let milestones =
                LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(milestone_sink));
            if let Err(error) = milestones.model_started(requested_model_profile_id).await {
                tracing::debug!(
                    kind = ?error.kind,
                    "loop model_started milestone failed before model request"
                );
            }
        }
    }

    async fn emit_model_completed(&self, effective_model_profile_id: ModelProfileId) {
        if let Some(milestone_sink) = &self.milestone_sink {
            let milestones =
                LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(milestone_sink));
            if let Err(error) = milestones.model_completed(effective_model_profile_id).await {
                tracing::debug!(
                    kind = ?error.kind,
                    "loop model_completed milestone failed after successful model response"
                );
            }
        }
    }

    async fn emit_model_failed(&self, reason_kind: AgentLoopHostErrorKind) {
        if let Some(milestone_sink) = &self.milestone_sink {
            let milestones =
                LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(milestone_sink));
            if let Err(error) = milestones.model_failed(reason_kind).await {
                tracing::debug!(
                    kind = ?error.kind,
                    "loop model_failed milestone failed after model error"
                );
            }
        }
    }

    async fn resolve_model_messages(
        &self,
        requested_messages: Vec<LoopModelMessage>,
    ) -> Result<Vec<HostManagedModelMessage>, AgentLoopHostError> {
        let context = self
            .load_model_context_window(!requested_messages.is_empty())
            .await?;

        if requested_messages.is_empty() {
            let context_messages = prompt_context_budget::select_prompt_context_messages(
                context.messages,
                self.prompt_context_budget,
            );
            let mut messages = Vec::with_capacity(context_messages.len());
            for (message, _) in context_messages {
                let Some(content_ref) = message_ref_from_context(&message) else {
                    continue;
                };
                let tool_result_content = tool_result_content_for_context_message(&message)?;
                let image_parts = self.read_image_parts(&message.image_attachments).await;
                messages.push(HostManagedModelMessage {
                    role: model_role_for_kind(message.kind),
                    content: message.content,
                    content_ref,
                    tool_result_provider_call: message.tool_result_provider_call,
                    tool_result_content,
                    image_parts,
                });
            }
            return Ok(messages);
        }

        let mut messages_by_ref = context_messages_by_ref(context.messages);
        let mut missing_message_ids = Vec::new();
        let mut needs_summary_history_lookup = false;
        for message in &requested_messages {
            if messages_by_ref.contains_key(message.content_ref.as_str()) {
                continue;
            }
            if let Some(materialization_store) = self.instruction_materialization_store.as_ref()
                && materialization_store
                    .get_materialized_message(&self.run_context, &message.content_ref)?
                    .is_some()
            {
                continue;
            }
            if identity_context::is_identity_model_message_ref(&message.content_ref) {
                continue;
            }
            if skill_context::is_snippet_model_message_ref(&message.content_ref) {
                continue;
            }
            if is_summary_model_message_ref(&message.content_ref) {
                needs_summary_history_lookup = true;
                continue;
            }
            if let Ok(message_id) = message_id_from_ref(&message.content_ref) {
                missing_message_ids.push(message_id);
            }
        }
        let snippet_messages_by_ref = if requested_messages
            .iter()
            .any(|message| skill_context::is_snippet_model_message_ref(&message.content_ref))
        {
            self.instruction_snippet_messages_by_ref().await?
        } else {
            HashMap::new()
        };
        if !missing_message_ids.is_empty() {
            let context_messages = self
                .thread_service
                .load_context_messages(LoadContextMessagesRequest {
                    scope: self.thread_scope.clone(),
                    thread_id: self.run_context.thread_id.clone(),
                    message_ids: missing_message_ids,
                })
                .await
                .map_err(context_read_error)?;
            messages_by_ref.extend(context_messages_by_ref(context_messages.messages));
        }
        if needs_summary_history_lookup {
            let history = self
                .thread_service
                .list_thread_history(ThreadHistoryRequest {
                    scope: self.thread_scope.clone(),
                    thread_id: self.run_context.thread_id.clone(),
                })
                .await
                .map_err(context_read_error)?;
            messages_by_ref.extend(history_summaries_by_ref(history.summary_artifacts));
        }
        let mut resolved = Vec::with_capacity(requested_messages.len());
        for message in requested_messages {
            let requested_role = HostManagedModelMessageRole::from_loop_role(&message.role)?;
            // Priority 1: trusted identity files resolved by the configured host source.
            if identity_context::is_identity_model_message_ref(&message.content_ref) {
                let Some(source) = self.identity_context_source.as_deref() else {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "identity message ref is unavailable: no identity source configured for this host",
                    ));
                };
                if requested_role != HostManagedModelMessageRole::System {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "model message role does not match identity context",
                    ));
                }
                let content = source
                    .resolve_identity_message_content(&self.run_context, &message.content_ref)
                    .await
                    .map_err(HostIdentityContextBuildError::into_host_error)?
                    .ok_or_else(|| {
                        AgentLoopHostError::new(
                            AgentLoopHostErrorKind::InvalidInvocation,
                            "identity message ref is unavailable: source returned no content for this ref",
                        )
                    })?;
                resolved.push(HostManagedModelMessage {
                    role: HostManagedModelMessageRole::System,
                    content: content.content,
                    content_ref: message.content_ref,
                    tool_result_provider_call: None,
                    tool_result_content: None,
                    image_parts: Vec::new(),
                });
                continue;
            }

            if let Some(materialization_store) = self.instruction_materialization_store.as_ref()
                && let Some(materialized) = materialization_store
                    .get_materialized_message(&self.run_context, &message.content_ref)?
            {
                let materialized_role =
                    HostManagedModelMessageRole::from_loop_role(&materialized.role)?;
                if requested_role != materialized_role {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "model message role does not match materialized instruction context",
                    ));
                }
                resolved.push(HostManagedModelMessage {
                    role: materialized_role,
                    content: materialized.model_content,
                    content_ref: message.content_ref,
                    tool_result_provider_call: None,
                    tool_result_content: None,
                    image_parts: Vec::new(),
                });
                continue;
            }

            if let Some(snippet_message) = snippet_messages_by_ref.get(message.content_ref.as_str())
            {
                if requested_role != snippet_message.role {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "model message role does not match skill context snippet",
                    ));
                }
                resolved.push(snippet_message.clone());
                continue;
            }

            // Priority 3: durable transcript messages (context window + history).
            let context_message = messages_by_ref
                .get(message.content_ref.as_str())
                .ok_or_else(|| {
                    AgentLoopHostError::new(
                        AgentLoopHostErrorKind::InvalidInvocation,
                        "model message reference is unavailable",
                    )
                })?;
            let durable_role = model_role_for_kind(context_message.kind);
            if requested_role != durable_role {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "model message role does not match transcript message",
                ));
            }
            let image_parts = self
                .read_image_parts(&context_message.image_attachments)
                .await;
            resolved.push(HostManagedModelMessage {
                role: durable_role,
                content: context_message.content.clone(),
                content_ref: message.content_ref,
                tool_result_provider_call: context_message.tool_result_provider_call.clone(),
                tool_result_content: tool_result_content_for_context_message(context_message)?,
                image_parts,
            });
        }
        Ok(resolved)
    }

    async fn load_model_context_window(
        &self,
        allow_smaller_cached_context: bool,
    ) -> Result<ironclaw_threads::ContextWindow, AgentLoopHostError> {
        let started_at = ironclaw_observability::live_latency_started_at();
        if let Some(cache) = self.context_window_cache.as_ref()
            && let Some(context) = cache
                .take_matching(
                    &self.thread_scope,
                    &self.run_context.thread_id,
                    self.max_messages,
                    allow_smaller_cached_context,
                )
                .await
        {
            trace_loop_host_latency_ok(
                "model_context_cache_hit",
                &self.run_context,
                started_at,
                self.max_messages,
                context.messages.len(),
            );
            return Ok(context);
        }
        trace_loop_host_latency_ok(
            "model_context_cache_miss",
            &self.run_context,
            started_at,
            self.max_messages,
            0,
        );

        let started_at = ironclaw_observability::live_latency_started_at();
        let context = self
            .thread_service
            .load_context_window(LoadContextWindowRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                max_messages: self.max_messages,
            })
            .await
            .map_err(context_read_error)?;
        trace_loop_host_latency_ok(
            "model_context_load_window",
            &self.run_context,
            started_at,
            self.max_messages,
            context.messages.len(),
        );
        Ok(context)
    }

    async fn instruction_snippet_messages_by_ref(
        &self,
    ) -> Result<HashMap<String, HostManagedModelMessage>, AgentLoopHostError> {
        let Some(source) = self.skill_context_source.as_deref() else {
            return Ok(HashMap::new());
        };
        let mut snippets =
            skill_context::build_skill_instruction_snippets(source, &self.run_context).await?;
        sort_instruction_snippets_for_prompt(&mut snippets);
        let mut messages = HashMap::with_capacity(snippets.len());
        for (ordinal, snippet) in snippets.into_iter().enumerate() {
            let content_ref = skill_context::snippet_model_message_ref(
                &snippet.snippet_ref,
                &snippet.safe_summary,
                &snippet.model_content,
                ordinal,
            )?;
            messages.insert(
                content_ref.as_str().to_string(),
                HostManagedModelMessage {
                    role: HostManagedModelMessageRole::System,
                    content: snippet.model_content,
                    content_ref,
                    tool_result_provider_call: None,
                    tool_result_content: None,
                    image_parts: Vec::new(),
                },
            );
        }
        Ok(messages)
    }
}

/// Host-managed text-only model gateway. Implementations own provider selection,
/// profile policy, retry/circuit behavior, and sanitization.
#[async_trait]
pub trait HostManagedModelGateway: Send + Sync {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError>;

    async fn stream_model_with_progress(
        &self,
        request: HostManagedModelRequest,
        _sink: Arc<dyn HostManagedModelStreamSink>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.stream_model(request).await
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.stream_model(request).await
    }

    async fn stream_model_with_capabilities_and_progress(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
        _sink: Arc<dyn HostManagedModelStreamSink>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.stream_model_with_capabilities(request, capabilities)
            .await
    }

    /// Resolve a scope-specific gateway, if this gateway multiplexes by scope.
    /// Production gateways return None (identity) → host uses `self` unchanged.
    /// Test harnesses override this to route per-thread scripted gateways.
    fn resolve_for_scope(
        &self,
        _scope: &TurnScope,
    ) -> Option<std::sync::Arc<dyn HostManagedModelGateway>> {
        None
    }
}

#[async_trait::async_trait]
pub trait HostManagedModelStreamSink: Send + Sync {
    async fn safe_text_update(&self, safe_text: String);
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostManagedModelRequest {
    pub model_profile_id: ModelProfileId,
    /// Zero-based index into the gateway provider's ordered fallback chain.
    #[serde(default)]
    pub fallback_index: u32,
    pub messages: Vec<HostManagedModelMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<HostManagedModelRouteSnapshot>,
    pub run_id: TurnRunId,
    pub turn_id: TurnId,
}

/// Boundary alias for the route snapshot carried from turn/run state into
/// host-managed model requests. This intentionally preserves the turn-owned
/// wire shape across the loop-host boundary instead of defining a duplicate
/// snapshot DTO here.
pub type HostManagedModelRouteSnapshot = ironclaw_loop_contracts::LoopModelRouteSnapshot;

/// An image attachment read back as raw bytes, ready to become a multimodal
/// content part for a vision-capable model. The bytes are carried undecorated;
/// base64 / `data:` URL formatting is a provider concern the model gateway owns
/// (it turns each into a `ContentPart::ImageUrl` data URL) and only for a model
/// that actually accepts images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostManagedModelImagePart {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// Reads attachment bytes for the current turn so the model port can build
/// multimodal image parts. Host composition implements this over the
/// project-scoped workspace filesystem (the same authority that landed the
/// attachment) and injects it into the model port. Deliberately narrow — bytes
/// for one scoped `storage_key` — so it carries no provider/runtime authority.
#[async_trait::async_trait]
pub trait LoopAttachmentReadPort: Send + Sync {
    async fn read_attachment_bytes(
        &self,
        scope: &ironclaw_host_api::resource::ResourceScope,
        storage_key: &str,
    ) -> Result<Vec<u8>, LoopAttachmentReadError>;
}

/// Failure reading attachment bytes for the multimodal path. Non-fatal: the
/// model port skips the image (the text `<attachments>` pointer remains).
#[derive(Debug)]
pub enum LoopAttachmentReadError {
    NotFound,
    Forbidden,
    Backend(String),
}

impl std::fmt::Display for LoopAttachmentReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "attachment not found"),
            Self::Forbidden => write!(f, "attachment read forbidden"),
            Self::Backend(reason) => write!(f, "attachment read backend error: {reason}"),
        }
    }
}

impl std::error::Error for LoopAttachmentReadError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostManagedModelMessage {
    pub role: HostManagedModelMessageRole,
    pub content: String,
    pub content_ref: LoopMessageRef,
    #[serde(default, skip_serializing)]
    pub tool_result_provider_call: Option<ProviderToolCallReferenceEnvelope>,
    #[serde(default, skip)]
    pub tool_result_content: Option<HostManagedToolResultContent>,
    /// Raw image-attachment bytes for the multimodal path, populated for any
    /// message that carries landed images. The gateway encodes and attaches
    /// them only for a vision-capable model (text-only models discard them and
    /// keep the `<attachments>` text pointer). Not serialized (transient turn
    /// data).
    #[serde(default, skip)]
    pub image_parts: Vec<HostManagedModelImagePart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostManagedToolResultContent {
    Reference {
        envelope: ToolResultReferenceEnvelope,
    },
    Resolved {
        safe_summary: ToolResultSafeSummary,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostManagedModelMessageRole {
    System,
    User,
    Assistant,
    ToolResult,
}

impl HostManagedModelMessageRole {
    fn from_loop_role(role: &str) -> Result<Self, AgentLoopHostError> {
        match role {
            "system" => Ok(Self::System),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool_result_reference" => Ok(Self::ToolResult),
            _ => Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "model message role is unsupported",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostManagedModelResponse {
    pub safe_text_deltas: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safe_reasoning_deltas: Vec<String>,
    pub output: ParentLoopOutput,
    /// Provider-reported token usage. Forwarded to [`LoopModelResponse::usage`]
    /// by the inner port wrapper, so the budget accountant can record actual
    /// USD spend instead of the conservative reservation estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<LoopModelUsage>,
    /// Authoritative ordered-chain index used for this successful call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_fallback_index: Option<u32>,
}

impl HostManagedModelResponse {
    pub fn assistant_reply(content: impl Into<String>) -> Self {
        let content = content.into();
        let sanitized_content = sanitize_model_visible_text(content);
        Self {
            safe_text_deltas: vec![sanitized_content.clone()],
            safe_reasoning_deltas: Vec::new(),
            output: ParentLoopOutput::AssistantReply(AssistantReply {
                content: sanitized_content,
            }),
            usage: None,
            effective_fallback_index: Some(0),
        }
    }

    pub fn assistant_reply_with_reasoning(
        content: impl Into<String>,
        reasoning: Option<String>,
    ) -> Self {
        let mut response = Self::assistant_reply(content);
        response.safe_reasoning_deltas = sanitized_reasoning_deltas(reasoning);
        response
    }

    pub fn capability_calls(
        calls: Vec<ironclaw_loop_contracts::CapabilityCallCandidate>,
        safe_text_delta: impl Into<String>,
    ) -> Self {
        let safe_text_delta = sanitize_model_visible_text(safe_text_delta);
        Self {
            safe_text_deltas: if safe_text_delta.is_empty() {
                Vec::new()
            } else {
                vec![safe_text_delta]
            },
            safe_reasoning_deltas: Vec::new(),
            output: ParentLoopOutput::CapabilityCalls(calls),
            usage: None,
            effective_fallback_index: Some(0),
        }
    }

    pub fn capability_calls_with_reasoning(
        calls: Vec<ironclaw_loop_contracts::CapabilityCallCandidate>,
        safe_text_delta: impl Into<String>,
        reasoning: Option<String>,
    ) -> Self {
        let mut response = Self::capability_calls(calls, safe_text_delta);
        response.safe_reasoning_deltas = sanitized_reasoning_deltas(reasoning);
        response
    }

    /// Attach provider-reported token usage. Returns the response so call
    /// sites can chain into [`assistant_reply`] / [`capability_calls`].
    pub fn with_usage(mut self, usage: LoopModelUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_effective_fallback_index(mut self, fallback_index: u32) -> Self {
        self.effective_fallback_index = Some(fallback_index);
        self
    }
}

fn sanitized_reasoning_deltas(reasoning: Option<String>) -> Vec<String> {
    reasoning
        .map(sanitize_model_visible_text)
        .filter(|reasoning| !reasoning.is_empty())
        .into_iter()
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostManagedModelErrorKind {
    /// Caller-side misuse of the host model port (unknown tool, malformed request).
    InvalidRequest,
    /// The request was valid when built but no longer matches current host
    /// state. Callers may rebuild the prompt/surface and retry.
    StaleRequest,
    /// Provider/model output was structurally invalid for the active loop contract.
    /// This is model-side bad output, not caller misuse.
    #[serde(alias = "invalid_output")]
    InvalidOutput,
    /// The provider refused the completion because its content filter rejected
    /// the request or response. Distinct from host/profile policy denial so the
    /// loop can ask the model to rephrase exactly once.
    ContentFiltered,
    PolicyDenied,
    ConfigurationError,
    /// Generic host-side resource/capacity exhaustion. Provider model-call
    /// outcomes use the precise variants below.
    BudgetExceeded,
    SpendBudgetExceeded,
    ContextOverflow,
    OutputTruncated,
    BudgetApprovalRequired,
    /// Durable host-side resource accounting failed. This is an
    /// infrastructure failure, not a provider credit or configured-budget
    /// outcome, and must remain distinct while it crosses the model port.
    BudgetAccountingFailed,
    /// Provider credentials are missing, expired, or otherwise unavailable.
    CredentialUnavailable,
    /// Provider throttled the request. `retry_after_ms` carries the bounded
    /// provider instruction when present.
    RateLimited,
    /// Provider returned a typed upstream 5xx availability failure.
    ProviderUnavailable,
    Unavailable,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("host-managed model {kind:?}: {safe_summary}")]
pub struct HostManagedModelError {
    pub kind: HostManagedModelErrorKind,
    pub safe_summary: String,
    pub reason_kind: Option<AgentLoopHostErrorReasonKind>,
    pub gate_ref: Option<LoopGateRef>,
    /// Provider-supplied retry delay. Typed so the recovery strategy does not
    /// have to parse model-visible detail text.
    pub retry_after_ms: Option<u64>,
    /// Deterministic evidence that the provider chain has another configured
    /// route. Recovery may advance only when this is present.
    pub next_fallback_index: Option<u32>,
    /// Provider-reported usage for a call that consumed tokens before failing.
    pub usage: Option<LoopModelUsage>,
    /// Model-visible, secret-scrubbed raw cause (status line, provider body
    /// snippet). Unlike `safe_summary`, this carries the original message so the
    /// failure explainer can describe the real fault. Secret VALUES must be
    /// redacted by the producer via
    /// [`ironclaw_loop_contracts::sanitize_model_visible_text`]; the
    /// summary word/delimiter ban is NOT applied here.
    pub detail: Option<String>,
}

impl HostManagedModelError {
    pub fn new(kind: HostManagedModelErrorKind, _raw_detail: impl Into<String>) -> Self {
        Self {
            kind,
            safe_summary: safe_model_summary(kind).to_string(),
            reason_kind: None,
            gate_ref: None,
            retry_after_ms: None,
            next_fallback_index: None,
            usage: None,
            detail: None,
        }
    }

    pub fn safe(kind: HostManagedModelErrorKind, safe_summary: impl Into<String>) -> Self {
        Self {
            kind,
            safe_summary: safe_summary.into(),
            reason_kind: None,
            gate_ref: None,
            retry_after_ms: None,
            next_fallback_index: None,
            usage: None,
            detail: None,
        }
    }

    /// Attach a secret-scrubbed model-visible detail. The caller is responsible
    /// for scrubbing secret VALUES first (see [`Self::detail`]).
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Attach a model-visible detail, hardening it via
    /// [`crate::scrub_model_visible_detail`] first: secret VALUES are redacted
    /// through the full leak-detector registry + prefix matcher, and any
    /// surviving injection payload is fenced as untrusted external content. Use
    /// when the raw cause has not already been sanitized (e.g. a provider HTTP
    /// body).
    pub fn safe_with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(crate::scrub_model_visible_detail(detail));
        self
    }

    pub fn with_reason_kind(mut self, reason_kind: AgentLoopHostErrorReasonKind) -> Self {
        self.reason_kind = Some(reason_kind);
        self
    }

    pub fn with_gate_ref(mut self, gate_ref: LoopGateRef) -> Self {
        self.gate_ref = Some(gate_ref);
        self
    }

    pub fn with_retry_after(mut self, retry_after: std::time::Duration) -> Self {
        self.retry_after_ms = Some(
            retry_after
                .as_millis()
                .min(u64::MAX as u128)
                .try_into()
                .unwrap_or(u64::MAX),
        );
        self
    }

    pub fn with_next_fallback_index(mut self, fallback_index: u32) -> Self {
        self.next_fallback_index = Some(fallback_index);
        self
    }

    pub fn with_usage(mut self, usage: LoopModelUsage) -> Self {
        self.usage = Some(usage);
        self
    }
}

fn validate_thread_scope_for_run(
    thread_scope: &ThreadScope,
    run_context: &LoopRunContext,
) -> Result<(), AgentLoopHostError> {
    if thread_scope.tenant_id != run_context.scope.tenant_id
        || Some(thread_scope.agent_id.clone()) != run_context.scope.agent_id
        || thread_scope.project_id != run_context.scope.project_id
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::ScopeMismatch,
            "thread scope does not match loop run scope",
        ));
    }
    // The thread store keys threads by `owner_user_id` (via the MountView in
    // `ThreadScope::to_resource_scope`), but that axis is absent from the
    // on-disk thread path, so a wrong owner silently reads an empty subtree
    // and surfaces as `UnknownThread`. Explicit-owner runs intentionally allow
    // actor/subject divergence for shared conversation routes, but the explicit
    // owner must still match the resolved thread owner. Legacy actor-fallback
    // runs continue to require owner=actor.
    if run_context.scope.has_explicit_thread_owner() {
        if run_context.scope.explicit_owner_user_id() != thread_scope.owner_user_id.as_ref() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "thread scope owner does not match the explicit loop run subject",
            ));
        }
    } else if let (Some(thread_owner), Some(actor)) =
        (thread_scope.owner_user_id.as_ref(), run_context.actor())
        && thread_owner != &actor.user_id
    {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::ScopeMismatch,
            "thread scope owner does not match the loop run actor",
        ));
    }
    Ok(())
}

fn bounded_limit(requested: usize, configured: usize) -> usize {
    let configured = configured.max(1);
    if requested == 0 {
        configured
    } else {
        requested.min(configured)
    }
}

fn validate_context_cursor(
    cursor: Option<&LoopInputCursor>,
    run_context: &LoopRunContext,
) -> Result<(), AgentLoopHostError> {
    if let Some(cursor) = cursor {
        if !cursor.is_for_run(run_context) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "context cursor does not belong to this loop run",
            ));
        }
        if cursor != &LoopInputCursor::origin_for_run(run_context) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "context cursor snapshots are not supported by this host",
            ));
        }
    }
    Ok(())
}

fn context_messages_by_ref(messages: Vec<ContextMessage>) -> HashMap<String, ContextMessage> {
    messages
        .into_iter()
        .filter_map(|message| {
            message_ref_from_context(&message)
                .map(|message_ref| (message_ref.as_str().to_string(), message))
        })
        .collect()
}

fn history_summaries_by_ref(summaries: Vec<SummaryArtifact>) -> HashMap<String, ContextMessage> {
    summaries
        .into_iter()
        .filter_map(|summary| {
            let context_message = ContextMessage {
                message_id: None,
                summary_id: Some(summary.summary_id),
                sequence: summary.end_sequence,
                kind: MessageKind::Summary,
                tool_result_provider_call: None,
                content: summary.content,
                image_attachments: Vec::new(),
            };
            message_ref_from_context(&context_message)
                .map(|message_ref| (message_ref.as_str().to_string(), context_message))
        })
        .collect()
}

fn context_message_to_compaction_metadata(
    message: &ContextMessage,
) -> Option<LoopContextCompactionMetadata> {
    message_ref_from_context(message)?;
    Some(LoopContextCompactionMetadata {
        sequence: message.sequence,
        kind: compaction_kind_for_message(message.kind),
        estimated_tokens: estimate_tokens_from_chars(&message.content).as_u64(),
    })
}

fn context_message_to_loop_message(
    selected: prompt_context_budget::SelectedPromptContextMessage,
) -> Option<LoopContextMessage> {
    let (message, estimated_tokens) = selected;
    let message_ref = message_ref_from_context(&message)?;
    let compaction = Some(LoopContextCompactionMetadata {
        sequence: message.sequence,
        kind: compaction_kind_for_message(message.kind),
        estimated_tokens,
    });
    Some(LoopContextMessage {
        message_ref: Some(message_ref),
        role: role_for_kind(message.kind).to_string(),
        safe_summary: safe_context_summary(message.kind).to_string(),
        compaction,
    })
}

fn compaction_kind_for_message(kind: MessageKind) -> LoopContextCompactionKind {
    match kind {
        MessageKind::User => LoopContextCompactionKind::User,
        MessageKind::Assistant => LoopContextCompactionKind::Assistant,
        MessageKind::System => LoopContextCompactionKind::System,
        MessageKind::Summary => LoopContextCompactionKind::Summary,
        MessageKind::CheckpointReference
        | MessageKind::ToolResultReference
        | MessageKind::CapabilityDisplayPreview => LoopContextCompactionKind::Other,
    }
}

fn message_ref_from_context(message: &ContextMessage) -> Option<LoopMessageRef> {
    if let Some(message_id) = message.message_id {
        return message_ref(message_id).ok();
    }
    message.summary_id.and_then(|summary_id| {
        LoopMessageRef::new(format!("msg:summary-{summary_id}"))
            .map_err(|_| ())
            .ok()
    })
}

fn message_ref(message_id: ThreadMessageId) -> Result<LoopMessageRef, AgentLoopHostError> {
    LoopMessageRef::new(format!("msg:{message_id}")).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "thread message reference could not be represented",
        )
    })
}

fn is_summary_model_message_ref(message_ref: &LoopMessageRef) -> bool {
    message_ref.as_str().starts_with("msg:summary-")
}

fn message_id_from_ref(
    message_ref: &LoopMessageRef,
) -> Result<ThreadMessageId, AgentLoopHostError> {
    let raw = message_ref
        .as_str()
        .strip_prefix("msg:")
        .ok_or_else(invalid_transcript_ref_error)?;
    ThreadMessageId::parse(raw).map_err(|_| invalid_transcript_ref_error())
}

fn invalid_transcript_ref_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        "transcript message reference is invalid",
    )
}

fn provider_call_reference_to_envelope(
    provider_call: ironclaw_loop_contracts::ProviderToolCallReference,
) -> ProviderToolCallReferenceEnvelope {
    let capability_id = provider_call.capability_id;
    let replay = provider_call.replay;
    ProviderToolCallReferenceEnvelope {
        provider_id: replay.provider_id,
        provider_model_id: replay.provider_model_id,
        provider_turn_id: replay.provider_turn_id,
        provider_call_id: replay.provider_call_id,
        provider_tool_name: replay.provider_tool_name,
        capability_id,
        arguments: replay.arguments,
        response_reasoning: replay.response_reasoning,
        reasoning: replay.reasoning,
        signature: replay.signature,
    }
}

fn role_for_kind(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::User => "user",
        MessageKind::Assistant => "assistant",
        MessageKind::System | MessageKind::Summary | MessageKind::CheckpointReference => {
            LOOP_SYSTEM_ROLE
        }
        MessageKind::ToolResultReference => "tool_result_reference",
        MessageKind::CapabilityDisplayPreview => "capability_display_preview",
    }
}

fn model_role_for_kind(kind: MessageKind) -> HostManagedModelMessageRole {
    match kind {
        MessageKind::User => HostManagedModelMessageRole::User,
        MessageKind::Assistant => HostManagedModelMessageRole::Assistant,
        MessageKind::System | MessageKind::Summary | MessageKind::CheckpointReference => {
            HostManagedModelMessageRole::System
        }
        MessageKind::ToolResultReference => HostManagedModelMessageRole::ToolResult,
        MessageKind::CapabilityDisplayPreview => HostManagedModelMessageRole::System,
    }
}

fn tool_result_content_for_context_message(
    message: &ContextMessage,
) -> Result<Option<HostManagedToolResultContent>, AgentLoopHostError> {
    if message.kind != MessageKind::ToolResultReference {
        return Ok(None);
    }
    let envelope =
        ToolResultReferenceEnvelope::from_json_str(&message.content).map_err(|error| {
            raw_agent_loop_host_error(
                "model_context",
                "decode_tool_result_reference",
                AgentLoopHostErrorKind::InvalidInvocation,
                "tool result reference transcript content is invalid",
                error,
            )
        })?;
    Ok(Some(HostManagedToolResultContent::Reference { envelope }))
}

fn safe_context_summary(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::User => "user message available",
        MessageKind::Assistant => "assistant message available",
        MessageKind::System => "system message available",
        MessageKind::Summary => "summary artifact available",
        MessageKind::CheckpointReference => "checkpoint reference available",
        MessageKind::ToolResultReference => "tool result reference available",
        MessageKind::CapabilityDisplayPreview => "capability display preview available",
    }
}

fn empty_surface_version() -> Result<CapabilitySurfaceVersion, AgentLoopHostError> {
    CapabilitySurfaceVersion::new(EMPTY_SURFACE_VERSION).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "empty capability surface version is invalid",
        )
    })
}

fn empty_capability_error() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        "no capabilities are available to this loop",
    )
}

fn context_read_error(error: SessionThreadError) -> AgentLoopHostError {
    raw_agent_loop_host_error(
        "thread_context",
        "read_context",
        AgentLoopHostErrorKind::Unavailable,
        "thread context is unavailable",
        error,
    )
}

fn transcript_write_error(error: SessionThreadError) -> AgentLoopHostError {
    // Log only the closed owner-defined variant name. The error message may
    // contain raw transcript content or storage credentials and must never be
    // forwarded or formatted here.
    let error_kind = error.kind_name();
    tracing::debug!(error_kind, "transcript write failed");
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::TranscriptWriteFailed,
        "assistant transcript write failed",
    )
}

fn model_gateway_error(error: HostManagedModelError) -> AgentLoopHostError {
    // Phase 2: the host-managed *model provider* summary is the highest-leak-risk
    // error string in the system — provider errors routinely embed prompt text,
    // tool input, host paths, and keys. When it fails strict card validation the
    // summary degrades to a fixed category sentence, but the rejected cause is no
    // longer dropped: it rides the model-visible detail channel through the
    // hardened scrubber (`scrub_model_visible_detail` — full leak-detector
    // registry redaction of secret VALUES + injection fencing). Descriptive cause
    // text (paths, status codes) survives so the failure explainer can describe
    // the real fault; secret values and injection payloads do not. A genuine
    // structured `error.detail`, which producers already scrub deliberately, wins
    // over the rejected-summary fallback.
    let (safe_summary, rejected_summary_detail) =
        match LoopSafeSummary::new(error.safe_summary.clone()) {
            Ok(_) => (error.safe_summary, None),
            Err(_) => {
                tracing::debug!("host-managed model summary rejected; using fixed fallback");
                (
                    safe_model_summary(error.kind).to_string(),
                    Some(scrub_model_visible_detail(error.safe_summary)),
                )
            }
        };
    let mut host_error = AgentLoopHostError::new(model_error_kind(error.kind), safe_summary);
    if let Some(reason_kind) = error.reason_kind {
        host_error = host_error.with_reason_kind(reason_kind);
    }
    if let Some(gate_ref) = error.gate_ref {
        host_error = host_error.with_gate_ref(gate_ref);
    }
    if let Some(retry_after_ms) = error.retry_after_ms {
        host_error = host_error.with_retry_after_ms(retry_after_ms);
    }
    if let Some(next_fallback_index) = error.next_fallback_index {
        host_error = host_error.with_next_fallback_index(next_fallback_index);
    }
    if let Some(usage) = error.usage {
        host_error = host_error.with_usage(usage);
    }
    // `error.detail` is already producer-scrubbed; fall back to the scrubbed
    // rejected summary only when there is no structured detail.
    if let Some(detail) = error.detail.or(rejected_summary_detail) {
        host_error = host_error.with_detail(detail);
    }
    host_error
}

fn model_error_kind(kind: HostManagedModelErrorKind) -> AgentLoopHostErrorKind {
    match kind {
        HostManagedModelErrorKind::InvalidRequest => AgentLoopHostErrorKind::InvalidInvocation,
        HostManagedModelErrorKind::StaleRequest => AgentLoopHostErrorKind::StaleSurface,
        HostManagedModelErrorKind::InvalidOutput => AgentLoopHostErrorKind::InvalidOutput,
        HostManagedModelErrorKind::ContentFiltered => AgentLoopHostErrorKind::ContentFiltered,
        HostManagedModelErrorKind::PolicyDenied => AgentLoopHostErrorKind::PolicyDenied,
        HostManagedModelErrorKind::ConfigurationError => AgentLoopHostErrorKind::Unavailable,
        HostManagedModelErrorKind::BudgetExceeded => AgentLoopHostErrorKind::BudgetExceeded,
        HostManagedModelErrorKind::SpendBudgetExceeded => {
            AgentLoopHostErrorKind::SpendBudgetExceeded
        }
        HostManagedModelErrorKind::ContextOverflow => AgentLoopHostErrorKind::ContextOverflow,
        HostManagedModelErrorKind::OutputTruncated => AgentLoopHostErrorKind::OutputTruncated,
        HostManagedModelErrorKind::BudgetApprovalRequired => {
            AgentLoopHostErrorKind::BudgetApprovalRequired
        }
        HostManagedModelErrorKind::BudgetAccountingFailed => {
            AgentLoopHostErrorKind::BudgetAccountingFailed
        }
        HostManagedModelErrorKind::CredentialUnavailable => {
            AgentLoopHostErrorKind::CredentialUnavailable
        }
        HostManagedModelErrorKind::RateLimited => AgentLoopHostErrorKind::RateLimited,
        HostManagedModelErrorKind::ProviderUnavailable => AgentLoopHostErrorKind::Unavailable,
        HostManagedModelErrorKind::Unavailable => AgentLoopHostErrorKind::Unavailable,
        HostManagedModelErrorKind::Cancelled => AgentLoopHostErrorKind::Cancelled,
    }
}

fn safe_model_summary(kind: HostManagedModelErrorKind) -> &'static str {
    match kind {
        HostManagedModelErrorKind::InvalidRequest => "model request is invalid",
        HostManagedModelErrorKind::StaleRequest => "model request surface is stale",
        HostManagedModelErrorKind::InvalidOutput => "model output was structurally invalid",
        HostManagedModelErrorKind::ContentFiltered => "model completion was content filtered",
        HostManagedModelErrorKind::PolicyDenied => "model profile is not permitted",
        HostManagedModelErrorKind::ConfigurationError => "model route configuration is invalid",
        HostManagedModelErrorKind::BudgetExceeded => "model request exceeded its budget",
        HostManagedModelErrorKind::SpendBudgetExceeded => {
            "model request exceeded its configured spend budget"
        }
        HostManagedModelErrorKind::ContextOverflow => {
            "model request exceeded the provider context window"
        }
        HostManagedModelErrorKind::OutputTruncated => {
            "model response was truncated before completion"
        }
        HostManagedModelErrorKind::BudgetApprovalRequired => "model request needs budget approval",
        HostManagedModelErrorKind::BudgetAccountingFailed => {
            "resource accounting storage is unavailable"
        }
        HostManagedModelErrorKind::CredentialUnavailable => "model credentials are unavailable",
        HostManagedModelErrorKind::RateLimited => "model provider rate limited the request",
        HostManagedModelErrorKind::ProviderUnavailable => {
            "model provider is temporarily unavailable"
        }
        HostManagedModelErrorKind::Unavailable => "model service is unavailable",
        HostManagedModelErrorKind::Cancelled => "model request was cancelled",
    }
}

#[cfg(test)]
mod tests {
    use crate::memory_context::latest_user_message_text;

    use super::*;

    #[test]
    fn missing_model_route_evidence_stays_explicit_after_deserialization() {
        let response = HostManagedModelResponse::assistant_reply("ok");
        let mut serialized = serde_json::to_value(response).expect("response serializes");
        serialized
            .as_object_mut()
            .expect("response is an object")
            .remove("effective_fallback_index");

        let decoded: HostManagedModelResponse =
            serde_json::from_value(serialized).expect("legacy response shape deserializes");

        assert_eq!(decoded.effective_fallback_index, None);
    }

    #[test]
    fn failed_model_usage_survives_host_error_mapping() {
        let usage = LoopModelUsage {
            input_tokens: 11,
            output_tokens: 7,
            ..Default::default()
        };

        let mapped = model_gateway_error(
            HostManagedModelError::safe(
                HostManagedModelErrorKind::OutputTruncated,
                "model response was truncated before completion",
            )
            .with_usage(usage),
        );

        assert_eq!(mapped.usage, Some(usage));
    }

    #[test]
    fn typed_provider_errors_reach_distinct_loop_recovery_classes_with_retry_payload() {
        for (kind, expected) in [
            (
                HostManagedModelErrorKind::RateLimited,
                AgentLoopHostErrorKind::RateLimited,
            ),
            (
                HostManagedModelErrorKind::ProviderUnavailable,
                AgentLoopHostErrorKind::Unavailable,
            ),
        ] {
            let mut error = HostManagedModelError::safe(kind, safe_model_summary(kind))
                .with_retry_after(std::time::Duration::from_millis(1_750));
            if kind == HostManagedModelErrorKind::ProviderUnavailable {
                error = error.with_next_fallback_index(1);
            }
            let mapped = model_gateway_error(error);

            assert_eq!(
                mapped.kind, expected,
                "{kind:?} must reach its distinct loop recovery class"
            );
            assert_eq!(
                mapped.retry_after_ms,
                Some(1_750),
                "{kind:?} must preserve its typed provider retry hint"
            );
            assert_eq!(
                mapped.next_fallback_index,
                (kind == HostManagedModelErrorKind::ProviderUnavailable).then_some(1),
                "{kind:?} must preserve only applicable fallback route evidence"
            );
        }
    }

    fn ctx_msg(sequence: u64, kind: MessageKind, content: &str) -> ContextMessage {
        ContextMessage {
            message_id: None,
            summary_id: None,
            sequence,
            kind,
            tool_result_provider_call: None,
            content: content.to_string(),
            image_attachments: Vec::new(),
        }
    }

    #[test]
    fn transcript_write_error_exposes_only_the_fixed_safe_cause() {
        let secret = concat!("sk-", "TRANSCRIPT0123456789SECRET");
        let raw_reply = "raw assistant reply that was never persisted";
        let mapped = transcript_write_error(SessionThreadError::Backend(format!(
            "write rejected for {raw_reply:?} using {secret}"
        )));

        assert_eq!(mapped.kind, AgentLoopHostErrorKind::TranscriptWriteFailed);
        assert_eq!(mapped.safe_summary, "assistant transcript write failed");
        assert_eq!(mapped.detail, None);
        let rendered = format!("{mapped:?}");
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains(raw_reply));
    }

    #[test]
    fn transcript_retry_delay_is_total_for_extreme_attempts() {
        assert_eq!(
            transcript_write_retry_delay("run:extreme-attempt", usize::MAX),
            Duration::from_millis(u64::MAX)
        );
    }

    /// CR review: `latest_user_message_text` returns the latest NON-BLANK user
    /// message — a blank trailing user turn must not drop memory for the run when
    /// an earlier user turn has content, and non-user rows are skipped.
    #[test]
    fn latest_user_message_text_uses_latest_non_blank_user_turn() {
        // A blank newest user turn must fall back to the earlier non-blank one.
        let blank_trailing = vec![
            ctx_msg(1, MessageKind::User, "remember the launch is friday"),
            ctx_msg(2, MessageKind::User, "   \n  "),
        ];
        assert_eq!(
            latest_user_message_text(&blank_trailing).as_deref(),
            Some("remember the launch is friday"),
            "a blank trailing user turn must not drop the earlier non-blank one"
        );

        // All-blank user rows → None (nothing to query memory with).
        let all_blank = vec![
            ctx_msg(1, MessageKind::User, "   "),
            ctx_msg(2, MessageKind::User, ""),
        ];
        assert_eq!(latest_user_message_text(&all_blank), None);

        // The newest non-blank user turn wins over an older one.
        let two_users = vec![
            ctx_msg(1, MessageKind::User, "older"),
            ctx_msg(2, MessageKind::User, "newest"),
        ];
        assert_eq!(
            latest_user_message_text(&two_users).as_deref(),
            Some("newest")
        );

        // A newer non-user row is skipped in favor of the latest user turn.
        let user_then_assistant = vec![
            ctx_msg(1, MessageKind::User, "the user turn"),
            ctx_msg(2, MessageKind::Assistant, "model reply"),
        ];
        assert_eq!(
            latest_user_message_text(&user_then_assistant).as_deref(),
            Some("the user turn")
        );
    }

    #[test]
    fn model_gateway_error_threads_detail_into_host_error() {
        let error = HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model service is unavailable",
        )
        .with_detail("HTTP 404 model not found");

        let host_error = model_gateway_error(error);

        assert_eq!(
            host_error.detail.as_deref(),
            Some("HTTP 404 model not found")
        );
    }

    #[test]
    fn model_gateway_error_preserves_budget_accounting_failure_kind() {
        let error = HostManagedModelError::safe(
            HostManagedModelErrorKind::BudgetAccountingFailed,
            "resource accounting storage is unavailable",
        );

        let mapped = model_gateway_error(error);

        assert_eq!(mapped.kind, AgentLoopHostErrorKind::BudgetAccountingFailed);
        assert_eq!(
            mapped.safe_summary,
            "resource accounting storage is unavailable"
        );
    }

    #[test]
    fn model_gateway_error_preserves_precise_budget_and_token_limit_kinds() {
        for (gateway_kind, host_kind) in [
            (
                HostManagedModelErrorKind::SpendBudgetExceeded,
                AgentLoopHostErrorKind::SpendBudgetExceeded,
            ),
            (
                HostManagedModelErrorKind::ContextOverflow,
                AgentLoopHostErrorKind::ContextOverflow,
            ),
            (
                HostManagedModelErrorKind::OutputTruncated,
                AgentLoopHostErrorKind::OutputTruncated,
            ),
        ] {
            let mapped = model_gateway_error(HostManagedModelError::safe(
                gateway_kind,
                "model request could not complete",
            ));

            assert_eq!(mapped.kind, host_kind);
        }
    }

    #[test]
    fn model_gateway_error_preserves_stale_request_kind() {
        let error = HostManagedModelError::safe(
            HostManagedModelErrorKind::StaleRequest,
            "model request surface is stale",
        );

        let mapped = model_gateway_error(error);

        assert_eq!(mapped.kind, AgentLoopHostErrorKind::StaleSurface);
        assert_eq!(mapped.safe_summary, "model request surface is stale");
    }

    #[test]
    fn safe_with_detail_scrubs_credential_tokens() {
        let error = HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model service is unavailable",
        )
        .safe_with_detail("provider rejected api_key=sk-secretvalue for HTTP 401");

        let detail = error.detail.expect("detail present");
        assert!(!detail.contains("sk-secretvalue"));
        assert!(detail.contains("[redacted]"));
        assert!(detail.contains("HTTP 401"));
    }

    #[test]
    fn model_gateway_error_without_detail_leaves_host_detail_none() {
        let error = HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            "model service is unavailable",
        );

        let host_error = model_gateway_error(error);

        assert_eq!(host_error.detail, None);
    }

    #[test]
    fn model_gateway_error_sanitizes_raw_detail_without_losing_budget_gate() {
        let gate_ref = LoopGateRef::new("gate:budget-static-check").expect("gate ref");
        let raw_detail = format!(
            "provider 500: {} tool temporarily unavailable; api_key=secret; /private/path",
            "System"
        );

        let error = HostManagedModelError::new(
            HostManagedModelErrorKind::BudgetApprovalRequired,
            raw_detail.as_str(),
        )
        .with_gate_ref(gate_ref.clone());

        assert_eq!(error.safe_summary, "model request needs budget approval");
        assert!(!error.safe_summary.contains("System tool"));
        assert!(!error.safe_summary.contains("secret"));
        assert!(!error.safe_summary.contains("/private/path"));

        let host_error = model_gateway_error(error);

        assert_eq!(
            host_error.kind,
            AgentLoopHostErrorKind::BudgetApprovalRequired
        );
        assert_eq!(host_error.gate_ref, Some(gate_ref));
        assert_eq!(
            host_error.safe_summary,
            "model request needs budget approval"
        );
        assert!(!host_error.safe_summary.contains("System tool"));
        assert!(!host_error.safe_summary.contains("secret"));
        assert!(!host_error.safe_summary.contains("/private/path"));
    }

    #[test]
    fn model_gateway_error_carries_rejected_summary_as_scrubbed_detail() {
        // Phase 2 (item 4): a provider summary that fails strict card validation
        // (path/delimiter-bearing) is no longer dropped — it rides the
        // model-visible detail channel, secret VALUES redacted, injection fenced.
        let error = HostManagedModelError::safe(
            HostManagedModelErrorKind::Unavailable,
            concat!(
                "provider 500 at /host/route with ghp",
                "_012345678901234567890123456789012345",
                " body"
            ),
        );

        let host_error = model_gateway_error(error);

        // Card summary degrades to the fixed category sentence.
        assert_eq!(host_error.safe_summary, "model service is unavailable");
        let detail = host_error
            .detail
            .expect("rejected summary should ride detail");
        // Secret value redacted, descriptive cause (path) preserved.
        assert!(
            !detail.contains(concat!("ghp", "_012345678901234567890123456789012345", "")),
            "credential token must be redacted: {detail}"
        );
        assert!(
            detail.contains("/host/route"),
            "path must survive on detail: {detail}"
        );
    }

    #[test]
    fn model_visible_detail_path_survives_but_never_reaches_public_projection() {
        // Phase 2 (item 3): a detail carrying a path and a credential token must
        // (a) scrub the token at ingestion, (b) preserve the path to the model,
        // and (c) expose NEITHER on the public projection surface.
        let raw = concat!(
            "read_file failed at /workspace/config.json using \
                   ghp",
            "_012345678901234567890123456789012345",
            ""
        );
        let detail = scrub_model_visible_detail(raw);
        assert!(
            !detail.contains(concat!("ghp", "_012345678901234567890123456789012345", "")),
            "token must be scrubbed at ingestion: {detail}"
        );
        assert!(
            detail.contains("/workspace/config.json"),
            "path must survive: {detail}"
        );

        let failure = ironclaw_turns::SanitizedFailure::new("host_unavailable")
            .expect("category")
            .with_detail(detail.clone());
        assert_eq!(failure.detail(), Some(detail.as_str()));

        // Public projection strips the whole detail channel — neither the path
        // nor any redaction marker reaches the browser.
        let public = failure.public_projection();
        assert_eq!(public.detail(), None);
        assert_eq!(public.category(), "host_unavailable");
    }

    #[test]
    fn personal_context_admitted_summary_empty_paths_uses_count_only() {
        let summary = personal_context_admitted_summary(&[]).unwrap();

        assert_eq!(summary.as_str(), "personal context admitted count 0");
    }

    #[test]
    fn personal_context_admitted_summary_uses_safe_basenames_only() {
        let paths = vec![
            IdentityFileName::new("USER.md").unwrap(),
            IdentityFileName::new("context/assistant-directives.md").unwrap(),
        ];

        let summary = personal_context_admitted_summary(&paths).unwrap();

        assert_eq!(
            summary.as_str(),
            "personal context admitted count 2 sources USER.md assistant-directives.md"
        );
        assert!(!summary.as_str().contains("context/assistant-directives.md"));
        assert!(!summary.as_str().contains('/'));
        assert!(!summary.as_str().contains('\\'));
    }

    #[test]
    fn personal_context_source_label_drops_empty_and_separator_only_labels() {
        assert_eq!(
            personal_context_source_label(r"private\USER.md").as_deref(),
            Some("USER.md")
        );
        assert_eq!(
            personal_context_source_label("context/%2Fassistant-directives.md").as_deref(),
            Some("2Fassistant-directives.md")
        );
        assert_eq!(personal_context_source_label("///"), None);
    }

    /// Every recovery hint the loop can emit must survive persistence.
    ///
    /// The vocabulary now has a single home (`host_api`) and `ironclaw_threads`
    /// deserializes that enum rather than re-declaring its variants, so drift is
    /// structurally impossible. This test guards the seam anyway, because the
    /// *failure mode* is what makes it worth a test: an unaccepted value does
    /// not surface a validation error to anyone — the caller **drops the whole
    /// observation**, so the model loses the cause *and* the guidance and sees
    /// only a bare summary.
    ///
    /// That is not hypothetical. #6284 item 4 added six hints while the
    /// vocabulary was still duplicated by hand, missed the copy, and every
    /// denial it was meant to improve persisted with no observation at all.
    /// Crate-level tests all passed; only the persistence seam revealed it.
    ///
    /// This crate is the lowest one that sees both the emitting and persisting
    /// sides. Driven through the real envelope constructor, not the validator,
    /// so it pins what production actually stores.
    #[test]
    fn every_recovery_hint_the_loop_can_emit_survives_persistence() {
        use ironclaw_host_api::result_meta::{CapabilityRecoveryHint, SameCallRetryConstraint};
        use ironclaw_loop_contracts::{
            MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleToolObservation,
            ObservationTrust, ToolObservationDetail, ToolObservationStatus,
            ToolRecoveryObservation,
        };
        use ironclaw_threads::{ToolResultReferenceEnvelope, ToolResultSafeSummary};

        for hint in CapabilityRecoveryHint::ALL {
            let observation = ModelVisibleToolObservation {
                schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
                status: ToolObservationStatus::Error,
                summary: "The capability failed.".to_string(),
                detail: ToolObservationDetail::GenericFailure {
                    failure_kind: ironclaw_host_api::result_meta::FailureKind::PolicyDenied,
                    detail: None,
                },
                artifacts: Vec::new(),
                recovery: Some(
                    ToolRecoveryObservation::new(SameCallRetryConstraint::Forbidden, *hint)
                        // Exercise the optional delay field too: it is a key on
                        // the same object, and an unknown KEY is rejected the
                        // same silent way an unknown hint is.
                        .with_retry_after(Some(30_000)),
                ),
                trust: ObservationTrust::UntrustedToolOutput,
            };
            let value = serde_json::to_value(&observation).expect("observation serializes");

            let envelope = ToolResultReferenceEnvelope::new_best_effort_model_observation(
                "result:hint-conformance",
                ToolResultSafeSummary::new("The capability failed.").expect("summary"),
                Some(value),
            )
            .expect("envelope constructs");

            assert!(
                envelope.model_observation.is_some(),
                "recovery hint {hint:?} does not survive persistence, so storing it DROPS THE \
                 WHOLE OBSERVATION — the model would lose the cause and the guidance."
            );
        }
    }
}
