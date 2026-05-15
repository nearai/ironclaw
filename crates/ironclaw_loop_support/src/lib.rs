//! Loop support services for IronClaw Reborn.
//!
//! This crate adapts durable Reborn support boundaries (threads/transcripts plus
//! host-managed model gateways) into the narrow `AgentLoopHost` ports. It does
//! not own provider clients, tool dispatchers, secrets, or runtime handles.
#![warn(unreachable_pub)]

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

mod capability_allow_set;
mod capability_port;
mod capability_surface_filter;
mod skill_context;

pub use capability_allow_set::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
};
pub use capability_port::{
    HostRuntimeLoopCapabilityPort, HostRuntimeLoopCapabilityPortFactory,
    LoopCapabilityInputResolver, LoopCapabilityResultWriter, concurrency_hint_from_effects,
};
pub use capability_surface_filter::CapabilitySurfaceProfileFilter;
pub use skill_context::{
    HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextSource,
    build_skill_run_snapshot,
};

use tokio::sync::Mutex;

use async_trait::async_trait;
use ironclaw_threads::{
    AppendAssistantDraftRequest, ContextMessage, LoadContextWindowRequest, MessageContent,
    MessageKind, MessageStatus, SessionThreadError, SessionThreadService, SummaryArtifact,
    ThreadHistoryRequest, ThreadMessageId, ThreadMessageRecord, ThreadScope,
    UpdateAssistantDraftRequest,
};
use ironclaw_turns::{
    LoopMessageRef, TurnId, TurnRunId,
    run_profile::ModelProfileId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, AssistantReply, BeginAssistantDraft,
        CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityDenied,
        CapabilityDeniedReasonKind, CapabilityInvocation, CapabilityOutcome,
        CapabilitySurfaceVersion, FinalizeAssistantMessage, InstructionMaterializationStore,
        LoopContextBundle, LoopContextMessage, LoopContextPort, LoopContextRequest,
        LoopHostMilestoneEmitter, LoopHostMilestoneSink, LoopInputCursor, LoopModelMessage,
        LoopModelPort, LoopModelRequest, LoopModelResponse, LoopPromptBundleAuthority,
        LoopRunContext, LoopRunInfoPort, LoopSafeSummary, LoopTranscriptPort, ModelStreamChunk,
        ParentLoopOutput, UpdateAssistantDraft, VisibleCapabilityRequest, VisibleCapabilitySurface,
        sanitize_model_visible_text,
    },
};
use serde::{Deserialize, Serialize};

const EMPTY_SURFACE_VERSION: &str = "empty:v1";

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
        }
    }

    pub fn with_skill_context_source(mut self, source: Arc<dyn HostSkillContextSource>) -> Self {
        self.skill_context_source = Some(source);
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

        Ok(LoopContextBundle {
            identity_messages: Vec::new(),
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
    max_messages: usize,
    prompt_authority: LoopPromptBundleAuthority,
    milestone_sink: Option<Arc<dyn LoopHostMilestoneSink>>,
    skill_context_source: Option<Arc<dyn HostSkillContextSource>>,
    instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
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
            max_messages,
            prompt_authority: LoopPromptBundleAuthority::shared(),
            milestone_sink: None,
            skill_context_source: None,
            instruction_materialization_store: None,
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
            max_messages,
            prompt_authority: LoopPromptBundleAuthority::shared(),
            milestone_sink: Some(milestone_sink),
            skill_context_source: None,
            instruction_materialization_store: None,
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

    pub fn with_instruction_materialization_store(
        mut self,
        store: Arc<dyn InstructionMaterializationStore>,
    ) -> Self {
        self.instruction_materialization_store = Some(store);
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
        let resolved_messages = self.resolve_model_messages(prompt_grant.messages).await?;
        self.emit_model_started(requested_model_profile_id).await;
        let gateway_response = match self
            .gateway
            .stream_model(HostManagedModelRequest {
                model_profile_id: model_profile_id.clone(),
                messages: resolved_messages,
                surface_version: request.surface_version,
                resolved_model_route: self.run_context.resolved_model_route.clone(),
                run_id: self.run_context.run_id,
                turn_id: self.run_context.turn_id,
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let host_error = model_gateway_error(error);
                self.emit_model_failed(host_error.kind).await;
                return Err(host_error);
            }
        };

        self.emit_model_completed(model_profile_id.clone()).await;

        Ok(LoopModelResponse {
            chunks: gateway_response
                .safe_text_deltas
                .into_iter()
                .map(|safe_text_delta| ModelStreamChunk {
                    safe_text_delta: sanitize_model_visible_text(safe_text_delta),
                })
                .collect(),
            output: gateway_response.output,
            effective_model_profile_id: model_profile_id,
        })
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
                    })
                })
                .collect();
            return Ok(messages);
        }

        let mut messages_by_ref = context_messages_by_ref(context.messages);
        let mut needs_history_lookup = false;
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
            needs_history_lookup = true;
            break;
        }
        let snippet_messages_by_ref = if requested_messages
            .iter()
            .any(|message| skill_context::is_snippet_model_message_ref(&message.content_ref))
        {
            self.instruction_snippet_messages_by_ref().await?
        } else {
            HashMap::new()
        };
        if needs_history_lookup {
            let history = self
                .thread_service
                .list_thread_history(ThreadHistoryRequest {
                    scope: self.thread_scope.clone(),
                    thread_id: self.run_context.thread_id.clone(),
                })
                .await
                .map_err(context_read_error)?;
            messages_by_ref.extend(history_messages_by_ref(history.messages));
            messages_by_ref.extend(history_summaries_by_ref(history.summary_artifacts));
        }
        let mut resolved = Vec::with_capacity(requested_messages.len());
        for message in requested_messages {
            let requested_role = HostManagedModelMessageRole::from_loop_role(&message.role)?;
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
        let snippets =
            skill_context::build_skill_instruction_snippets(source, &self.run_context).await?;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostManagedModelMessageRole {
    System,
    User,
    Assistant,
}

impl HostManagedModelMessageRole {
    fn from_loop_role(role: &str) -> Result<Self, AgentLoopHostError> {
        match role {
            "system" | "tool_result_reference" => Ok(Self::System),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
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

fn history_messages_by_ref(messages: Vec<ThreadMessageRecord>) -> HashMap<String, ContextMessage> {
    messages
        .into_iter()
        .filter(|message| model_visible_status(message.status))
        .filter_map(|message| {
            let content = message.content?;
            let context_message = ContextMessage {
                message_id: Some(message.message_id),
                summary_id: None,
                sequence: message.sequence,
                kind: message.kind,
                content,
            };
            message_ref_from_context(&context_message)
                .map(|message_ref| (message_ref.as_str().to_string(), context_message))
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
                content: summary.content,
            };
            message_ref_from_context(&context_message)
                .map(|message_ref| (message_ref.as_str().to_string(), context_message))
        })
        .collect()
}

fn model_visible_status(status: MessageStatus) -> bool {
    matches!(
        status,
        MessageStatus::Accepted | MessageStatus::Submitted | MessageStatus::Finalized
    )
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

fn role_for_kind(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::User => "user",
        MessageKind::Assistant => "assistant",
        MessageKind::System | MessageKind::Summary | MessageKind::CheckpointReference => "system",
        MessageKind::ToolResultReference => "tool_result_reference",
    }
}

fn model_role_for_kind(kind: MessageKind) -> HostManagedModelMessageRole {
    match kind {
        MessageKind::User => HostManagedModelMessageRole::User,
        MessageKind::Assistant => HostManagedModelMessageRole::Assistant,
        MessageKind::System
        | MessageKind::Summary
        | MessageKind::CheckpointReference
        | MessageKind::ToolResultReference => HostManagedModelMessageRole::System,
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

fn context_read_error(_error: SessionThreadError) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::Unavailable,
        "thread context is unavailable",
    )
}

fn transcript_write_error(_error: SessionThreadError) -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::TranscriptWriteFailed,
        "assistant transcript write failed",
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
