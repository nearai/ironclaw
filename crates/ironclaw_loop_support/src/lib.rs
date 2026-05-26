//! Loop support services for IronClaw Reborn.
//!
//! This crate adapts durable Reborn support boundaries (threads/transcripts plus
//! host-managed model gateways) into the narrow `AgentLoopHost` ports. It does
//! not own provider clients, tool dispatchers, secrets, or runtime handles.
#![warn(unreachable_pub)]

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

mod budget_accountant;
mod cancellation_port;
mod capability_allow_set;
mod capability_info;
mod capability_port;
mod capability_surface_filter;
mod filesystem_checkpoint_state;
mod filesystem_skill_bundle_source;
pub mod identity_context;
mod input_port;
mod input_queue;
mod skill_bundle_context_source;
mod skill_bundle_source;
mod skill_context;
mod turn_event_publisher;

pub use budget_accountant::{
    BudgetSeedingPolicy, GovernorBackedAccountant, ModelCost, ModelCostTable, ZeroCostTable,
};
pub use cancellation_port::{
    AlwaysAliveLoopCancellationPort, AlwaysAliveRunCancellationFactory,
    CompositeTurnRunWakeNotifier, ProductLiveCancellationProbe, ProductLiveCancellationReadiness,
    RunCancellationFactory, RunCancellationHandle, RunCancellationObservationKind,
    RunStateLoopCancellationPort, TurnStateRunCancellationFactory,
    verify_product_live_cancellation_probe,
};
pub use capability_allow_set::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
};
pub use capability_port::{
    HostRuntimeLoopCapabilityPort, HostRuntimeLoopCapabilityPortFactory,
    LoopCapabilityInputResolver, LoopCapabilityResultWriter, concurrency_hint_from_effects,
    loop_driver_execution_extension_id,
};
pub use capability_surface_filter::{
    CapabilitySurfaceProfileFilter, CapabilitySurfaceVisibleFilter,
};
pub use filesystem_checkpoint_state::FilesystemCheckpointStateStore;
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
pub use skill_bundle_context_source::SkillBundleContextSource;
pub use skill_bundle_source::{
    SkillBundleDescriptor, SkillBundleId, SkillBundleProvenance, SkillBundleSource,
    SkillBundleSourceError, SkillFilePath, SkillSourceKind, sort_skill_bundle_descriptors,
};
pub use skill_context::{
    HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextSource,
    build_skill_run_snapshot,
};
pub use turn_event_publisher::EventPublishingTurnRunTransitionPort;

use tokio::sync::{Mutex, OnceCell};

use async_trait::async_trait;
use ironclaw_threads::{
    AppendAssistantDraftRequest, AppendToolResultReferenceRequest, ContextMessage,
    LoadContextMessagesRequest, LoadContextWindowRequest, MessageContent, MessageKind,
    MessageStatus, ProviderToolCallReferenceEnvelope, SessionThreadError, SessionThreadService,
    SummaryArtifact, ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    ToolResultSafeSummary, UpdateAssistantDraftRequest,
};
use ironclaw_turns::{
    LoopMessageRef, TurnId, TurnRunId,
    run_profile::ModelProfileId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AppendCapabilityResultRef, AssistantReply,
        BeginAssistantDraft, CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityDenied,
        CapabilityDeniedReasonKind, CapabilityInvocation, CapabilityOutcome,
        CapabilitySurfaceVersion, FinalizeAssistantMessage, InstructionMaterializationStore,
        LoopCapabilityPort, LoopContextBundle, LoopContextMessage, LoopContextPort,
        LoopContextRequest, LoopDriverNoteKind, LoopHostMilestoneEmitter, LoopHostMilestoneSink,
        LoopInputCursor, LoopModelBudgetAccountant, LoopModelMessage, LoopModelPort,
        LoopModelRequest, LoopModelResponse, LoopPromptBundleAuthority, LoopRunContext,
        LoopRunInfoPort, LoopSafeSummary, LoopTranscriptPort, ModelCallOutcome, ModelStreamChunk,
        ParentLoopOutput, PromptMode, UpdateAssistantDraft, VisibleCapabilityRequest,
        VisibleCapabilitySurface, sanitize_model_visible_text,
        sort_instruction_snippets_for_prompt,
    },
};
use serde::{Deserialize, Serialize};
const EMPTY_SURFACE_VERSION: &str = "empty:v1";
const LOOP_SYSTEM_ROLE: &str = "system";

pub fn raw_agent_loop_host_error(
    component: &'static str,
    operation: &'static str,
    kind: AgentLoopHostErrorKind,
    safe_summary: impl Into<String>,
    raw_detail: impl std::fmt::Display,
) -> AgentLoopHostError {
    let safe_summary = safe_summary.into();
    tracing::warn!(
        component,
        operation,
        kind = ?kind,
        safe_summary = %safe_summary,
        raw_detail = %raw_detail,
        "agent loop host error mapped to safe summary"
    );
    AgentLoopHostError::new(kind, safe_summary)
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
    identity_candidates: Arc<IdentityCandidateCache>,
    milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
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
            identity_candidates: Arc::new(IdentityCandidateCache::new()),
            milestone_sink: None,
        }
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
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
        let context = self
            .thread_service
            .load_context_window(LoadContextWindowRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                max_messages,
            })
            .await
            .map_err(context_read_error)?;

        let instruction_snippets = match self.skill_context_source.as_deref() {
            Some(source) => {
                skill_context::build_skill_instruction_snippets(source, &self.run_context).await?
            }
            None => Vec::new(),
        };
        let identity_messages = match self.identity_context_source.as_deref() {
            Some(source) => {
                let mode = request.mode;
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
                self.publish_personal_context_admitted(
                    mode,
                    &outcome.admitted_personal_context_paths,
                );
                outcome.messages
            }
            None => Vec::new(),
        };

        Ok(LoopContextBundle {
            identity_messages,
            messages: context
                .messages
                .into_iter()
                .filter_map(context_message_to_loop_message)
                .collect(),
            instruction_snippets,
            memory_snippets: Vec::new(),
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
    // Only successful milestone publications are recorded here: if best-effort
    // publishing fails after the transcript write, an idempotent retry can try again.
    emitted_assistant_reply_finalized_refs: Arc<Mutex<HashSet<String>>>,
}

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
            emitted_assistant_reply_finalized_refs: Arc::new(Mutex::new(HashSet::new())),
        }
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
        let reply_content = request.reply.content;
        let draft = self
            .thread_service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                turn_run_id: self.run_context.run_id.to_string(),
                content: MessageContent::text(reply_content.clone()),
            })
            .await
            .map_err(transcript_write_error)?;
        if draft.status == MessageStatus::Finalized {
            if draft.content.as_deref() == Some(reply_content.as_str()) {
                let message_ref = message_ref(draft.message_id)?;
                self.emit_assistant_reply_finalized(message_ref.clone())
                    .await?;
                return Ok(message_ref);
            }
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::TranscriptWriteFailed,
                "assistant transcript write failed",
            ));
        }
        let finalized = self
            .thread_service
            .finalize_assistant_message(
                &self.thread_scope,
                &self.run_context.thread_id,
                draft.message_id,
                MessageContent::text(reply_content.clone()),
            )
            .await;
        match finalized {
            Ok(message) => {
                let message_ref = message_ref(message.message_id)?;
                self.emit_assistant_reply_finalized(message_ref.clone())
                    .await?;
                Ok(message_ref)
            }
            Err(error) => {
                if let Some(message_id) = self
                    .already_finalized_matching_reply(draft.message_id, &reply_content)
                    .await?
                {
                    let message_ref = message_ref(message_id)?;
                    self.emit_assistant_reply_finalized(message_ref.clone())
                        .await?;
                    return Ok(message_ref);
                }
                Err(transcript_write_error(error))
            }
        }
    }

    async fn append_capability_result_ref(
        &self,
        request: AppendCapabilityResultRef,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        validate_thread_scope_for_run(&self.thread_scope, &self.run_context)?;
        let safe_summary = LoopSafeSummary::new(request.safe_summary).map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "tool result reference summary is not safe",
            )
        })?;
        let safe_summary =
            ToolResultSafeSummary::new(safe_summary.as_str().to_string()).map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "tool result reference summary is not safe",
                )
            })?;
        let record = self
            .thread_service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                turn_run_id: self.run_context.run_id.to_string(),
                result_ref: request.result_ref.as_str().to_string(),
                safe_summary,
                provider_call: request
                    .provider_call
                    .map(provider_call_reference_to_envelope),
            })
            .await
            .map_err(transcript_write_error)?;
        message_ref(record.message_id)
    }
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
                diagnostic_ref = ?error.diagnostic_ref,
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

    async fn already_finalized_matching_reply(
        &self,
        message_id: ThreadMessageId,
        reply_content: &str,
    ) -> Result<Option<ThreadMessageId>, AgentLoopHostError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
            })
            .await
            .map_err(transcript_write_error)?;
        let expected_run_id = self.run_context.run_id.to_string();
        Ok(history.messages.into_iter().find_map(|message| {
            let belongs_to_run = message.turn_run_id.as_deref() == Some(expected_run_id.as_str());
            let matches_reply = message.status == MessageStatus::Finalized
                && message.content.as_deref() == Some(reply_content);
            if message.message_id == message_id && belongs_to_run && matches_reply {
                Some(message.message_id)
            } else {
                None
            }
        }))
    }
}

/// Empty capability surface for the text-only loop-support MVP.
#[derive(Debug, Clone, Default)]
pub struct EmptyLoopCapabilityPort;

#[async_trait]
impl ironclaw_turns::run_profile::LoopCapabilityPort for EmptyLoopCapabilityPort {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            version: empty_surface_version()?,
            descriptors: Vec::new(),
        })
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
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
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
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
        let outcomes = request
            .invocations
            .into_iter()
            .map(|_| {
                CapabilityOutcome::Denied(CapabilityDenied {
                    reason_kind: CapabilityDeniedReasonKind::EmptySurface,
                    safe_summary: "no capabilities are available to this loop".to_string(),
                })
            })
            .collect();
        Ok(CapabilityBatchOutcome {
            outcomes,
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
    prompt_authority: LoopPromptBundleAuthority,
    milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
    identity_context_source: Option<Arc<dyn HostIdentityContextSource>>,
    /// Optional budget accountant. When set, every model call goes through
    /// `pre_model_call` (reserve) → gateway → `post_model_call`
    /// (reconcile/release). When `None`, no budget enforcement happens —
    /// this preserves v1-era behavior for hosts that have not opted in.
    budget_accountant: Option<Arc<dyn LoopModelBudgetAccountant>>,
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
            prompt_authority: LoopPromptBundleAuthority::shared(),
            milestone_sink: None,
            skill_context_source: None,
            instruction_materialization_store: None,
            identity_context_source: None,
            budget_accountant: None,
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
            prompt_authority: LoopPromptBundleAuthority::shared(),
            milestone_sink: Some(milestone_sink),
            skill_context_source: None,
            instruction_materialization_store: None,
            identity_context_source: None,
            budget_accountant: None,
        }
    }

    /// Inject a budget accountant. Each `stream_model` call passes through
    /// `pre_model_call` and `post_model_call` so a [`GovernorBackedAccountant`]
    /// (or any custom impl) can reserve/reconcile against the resource
    /// governor.
    pub fn with_budget_accountant(
        mut self,
        accountant: Arc<dyn LoopModelBudgetAccountant>,
    ) -> Self {
        self.budget_accountant = Some(accountant);
        self
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

        // Resolve messages *before* the budget reservation so a message-
        // resolution failure here cannot orphan an active hold. The
        // reservation must only be taken once we know the call is going
        // to reach the provider.
        let resolved_messages = self.resolve_model_messages(prompt_grant.messages).await?;

        // Pre-call budget check: reserve estimated cost against the
        // governor cascade. A denial here short-circuits before any
        // provider/credential touch. Anything fallible after this point
        // must funnel through `finalize_post_model_call` so the hold is
        // reconciled or released.
        if let Some(accountant) = &self.budget_accountant
            && let Err(budget_error) = accountant.pre_model_call(&self.run_context, &request).await
        {
            return Err(budget_error.into_host_error());
        }
        self.emit_model_started(requested_model_profile_id).await;
        let host_request = HostManagedModelRequest {
            model_profile_id: model_profile_id.clone(),
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
            self.gateway
                .stream_model_with_capabilities(host_request, capabilities)
                .await
        } else {
            self.gateway.stream_model(host_request).await
        };

        let host_response_result = match gateway_result {
            Ok(response) => {
                let loop_response = LoopModelResponse {
                    chunks: response
                        .safe_text_deltas
                        .into_iter()
                        .map(|safe_text_delta| ModelStreamChunk {
                            safe_text_delta: sanitize_model_visible_text(safe_text_delta),
                        })
                        .collect(),
                    output: response.output,
                    effective_model_profile_id: model_profile_id.clone(),
                };
                Ok(loop_response)
            }
            Err(error) => Err(model_gateway_error(error)),
        };

        // Post-call accounting fires on BOTH success and failure paths so
        // the in-flight reservation never leaks.
        if let Some(accountant) = &self.budget_accountant {
            let outcome = match &host_response_result {
                Ok(response) => ModelCallOutcome::Success(response),
                Err(host_error) => {
                    // Construct a sanitized gateway error mirroring the
                    // host-error kind for the accountant. We do not expose
                    // the host_error's full diagnostic_ref to the
                    // accountant — the accountant only needs the kind.
                    let gateway_error = ironclaw_turns::run_profile::LoopModelGatewayError::new(
                        host_error.kind,
                        host_error.safe_summary.clone(),
                    )
                    .unwrap_or_else(|_| {
                        ironclaw_turns::run_profile::LoopModelGatewayError::new(
                            AgentLoopHostErrorKind::Internal,
                            "budget accountant invalid summary",
                        )
                        .unwrap_or_else(|_| {
                            panic!(
                                "internal budget-accountant invariant: cannot build safe summary"
                            )
                        })
                    });
                    return self
                        .finalize_post_model_call(
                            accountant,
                            &request,
                            ModelCallOutcome::Failure(&gateway_error),
                            host_response_result,
                        )
                        .await;
                }
            };
            if let Err(acc_error) = accountant
                .post_model_call(&self.run_context, &request, outcome)
                .await
            {
                self.emit_model_failed(acc_error.kind).await;
                return Err(acc_error.into_host_error());
            }
        }

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
    async fn finalize_post_model_call(
        &self,
        accountant: &Arc<dyn LoopModelBudgetAccountant>,
        request: &LoopModelRequest,
        outcome: ModelCallOutcome<'_>,
        host_response_result: Result<LoopModelResponse, AgentLoopHostError>,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        // Accountant contract: durable accounting/release failures must fail
        // closed. Swallowing on the provider-error path would hide stuck
        // reservations and misreport the failure cause, so the accountant
        // error takes precedence over the original model error.
        if let Err(acc_error) = accountant
            .post_model_call(&self.run_context, request, outcome)
            .await
        {
            tracing::warn!(
                kind = ?acc_error.kind,
                diagnostic_ref = ?acc_error.diagnostic_ref,
                "budget accountant post-call failed during error path; reporting as host error"
            );
            self.emit_model_failed(acc_error.kind).await;
            return Err(acc_error.into_host_error());
        }
        match host_response_result {
            Ok(response) => Ok(response),
            Err(error) => {
                self.emit_model_failed(error.kind).await;
                Err(error)
            }
        }
    }

    async fn emit_model_started(&self, requested_model_profile_id: Option<ModelProfileId>) {
        if let Some(milestone_sink) = &self.milestone_sink {
            let milestones =
                LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(milestone_sink));
            if let Err(error) = milestones.model_started(requested_model_profile_id).await {
                tracing::debug!(
                    kind = ?error.kind,
                    diagnostic_ref = ?error.diagnostic_ref,
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
                    diagnostic_ref = ?error.diagnostic_ref,
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
                    diagnostic_ref = ?error.diagnostic_ref,
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
            .thread_service
            .load_context_window(LoadContextWindowRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.run_context.thread_id.clone(),
                max_messages: self.max_messages,
            })
            .await
            .map_err(context_read_error)?;

        if requested_messages.is_empty() {
            let messages = context
                .messages
                .into_iter()
                .filter_map(|message| {
                    let content_ref = message_ref_from_context(&message)?;
                    Some(HostManagedModelMessage {
                        role: model_role_for_kind(message.kind),
                        content: message.content,
                        content_ref,
                        tool_result_provider_call: message.tool_result_provider_call,
                    })
                })
                .collect();
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
                    content: materialized.safe_content,
                    content_ref: message.content_ref,
                    tool_result_provider_call: None,
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
            resolved.push(HostManagedModelMessage {
                role: durable_role,
                content: context_message.content.clone(),
                content_ref: message.content_ref,
                tool_result_provider_call: context_message.tool_result_provider_call.clone(),
            });
        }
        Ok(resolved)
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
                ordinal,
            )?;
            messages.insert(
                content_ref.as_str().to_string(),
                HostManagedModelMessage {
                    role: HostManagedModelMessageRole::System,
                    content: snippet.safe_summary,
                    content_ref,
                    tool_result_provider_call: None,
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

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.stream_model(request).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostManagedModelRequest {
    pub model_profile_id: ModelProfileId,
    pub messages: Vec<HostManagedModelMessage>,
    pub surface_version: Option<CapabilitySurfaceVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<HostManagedModelRouteSnapshot>,
    pub run_id: TurnRunId,
    pub turn_id: TurnId,
}

/// Boundary alias for the route snapshot carried from turn/run state into
/// host-managed model requests. This intentionally preserves the turn-owned
/// wire shape across the loop-support boundary instead of defining a duplicate
/// snapshot DTO here.
pub type HostManagedModelRouteSnapshot = ironclaw_turns::run_profile::LoopModelRouteSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostManagedModelMessage {
    pub role: HostManagedModelMessageRole,
    pub content: String,
    pub content_ref: LoopMessageRef,
    #[serde(default, skip_serializing)]
    pub tool_result_provider_call: Option<ProviderToolCallReferenceEnvelope>,
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
    pub output: ParentLoopOutput,
}

impl HostManagedModelResponse {
    pub fn assistant_reply(content: impl Into<String>) -> Self {
        let content = content.into();
        let safe_content = sanitize_model_visible_text(content);
        Self {
            safe_text_deltas: vec![safe_content.clone()],
            output: ParentLoopOutput::AssistantReply(AssistantReply {
                content: safe_content,
            }),
        }
    }

    pub fn capability_calls(
        calls: Vec<ironclaw_turns::run_profile::CapabilityCallCandidate>,
        safe_text_delta: impl Into<String>,
    ) -> Self {
        let safe_text_delta = sanitize_model_visible_text(safe_text_delta);
        Self {
            safe_text_deltas: if safe_text_delta.is_empty() {
                Vec::new()
            } else {
                vec![safe_text_delta]
            },
            output: ParentLoopOutput::CapabilityCalls(calls),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostManagedModelErrorKind {
    InvalidRequest,
    PolicyDenied,
    ConfigurationError,
    BudgetExceeded,
    /// Provider credentials are missing, expired, or otherwise unavailable.
    CredentialUnavailable,
    Unavailable,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("host-managed model {kind:?}: {safe_summary}")]
pub struct HostManagedModelError {
    pub kind: HostManagedModelErrorKind,
    pub safe_summary: String,
}

impl HostManagedModelError {
    pub fn new(kind: HostManagedModelErrorKind, _raw_detail: impl Into<String>) -> Self {
        Self {
            kind,
            safe_summary: safe_model_summary(kind).to_string(),
        }
    }

    pub fn safe(kind: HostManagedModelErrorKind, safe_summary: impl Into<String>) -> Self {
        Self {
            kind,
            safe_summary: safe_summary.into(),
        }
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
            };
            message_ref_from_context(&context_message)
                .map(|message_ref| (message_ref.as_str().to_string(), context_message))
        })
        .collect()
}

fn context_message_to_loop_message(message: ContextMessage) -> Option<LoopContextMessage> {
    let message_ref = message_ref_from_context(&message)?;
    Some(LoopContextMessage {
        message_ref: Some(message_ref),
        role: role_for_kind(message.kind).to_string(),
        safe_summary: safe_context_summary(message.kind).to_string(),
    })
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
    provider_call: ironclaw_turns::run_profile::ProviderToolCallReference,
) -> ProviderToolCallReferenceEnvelope {
    ProviderToolCallReferenceEnvelope {
        provider_id: provider_call.provider_id,
        provider_model_id: provider_call.provider_model_id,
        provider_turn_id: provider_call.provider_turn_id,
        provider_call_id: provider_call.provider_call_id,
        provider_tool_name: provider_call.provider_tool_name,
        capability_id: provider_call.capability_id,
        arguments: provider_call.arguments,
        response_reasoning: provider_call.response_reasoning,
        reasoning: provider_call.reasoning,
        signature: provider_call.signature,
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
    }
}

fn safe_context_summary(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::User => "user message available",
        MessageKind::Assistant => "assistant message available",
        MessageKind::System => "system message available",
        MessageKind::Summary => "summary artifact available",
        MessageKind::CheckpointReference => "checkpoint reference available",
        MessageKind::ToolResultReference => "tool result reference available",
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
    raw_agent_loop_host_error(
        "thread_transcript",
        "write_transcript",
        AgentLoopHostErrorKind::TranscriptWriteFailed,
        "assistant transcript write failed",
        error,
    )
}

fn model_gateway_error(error: HostManagedModelError) -> AgentLoopHostError {
    let safe_summary = if LoopSafeSummary::new(error.safe_summary.clone()).is_ok() {
        error.safe_summary
    } else {
        safe_model_summary(error.kind).to_string()
    };
    AgentLoopHostError::new(model_error_kind(error.kind), safe_summary)
}

fn model_error_kind(kind: HostManagedModelErrorKind) -> AgentLoopHostErrorKind {
    match kind {
        HostManagedModelErrorKind::InvalidRequest => AgentLoopHostErrorKind::InvalidInvocation,
        HostManagedModelErrorKind::PolicyDenied => AgentLoopHostErrorKind::PolicyDenied,
        HostManagedModelErrorKind::ConfigurationError => AgentLoopHostErrorKind::Unavailable,
        HostManagedModelErrorKind::BudgetExceeded => AgentLoopHostErrorKind::BudgetExceeded,
        HostManagedModelErrorKind::CredentialUnavailable => {
            AgentLoopHostErrorKind::CredentialUnavailable
        }
        HostManagedModelErrorKind::Unavailable => AgentLoopHostErrorKind::Unavailable,
        HostManagedModelErrorKind::Cancelled => AgentLoopHostErrorKind::Cancelled,
    }
}

fn safe_model_summary(kind: HostManagedModelErrorKind) -> &'static str {
    match kind {
        HostManagedModelErrorKind::InvalidRequest => "model request is invalid",
        HostManagedModelErrorKind::PolicyDenied => "model profile is not permitted",
        HostManagedModelErrorKind::ConfigurationError => "model route configuration is invalid",
        HostManagedModelErrorKind::BudgetExceeded => "model request exceeded its budget",
        HostManagedModelErrorKind::CredentialUnavailable => "model credentials are unavailable",
        HostManagedModelErrorKind::Unavailable => "model service is unavailable",
        HostManagedModelErrorKind::Cancelled => "model request was cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
