use std::sync::Arc;

use async_trait::async_trait;

use super::host::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilitySurfaceVersion, LoopContextBundle,
    LoopContextMessage, LoopContextPort, LoopContextRequest, LoopModelMessage, LoopPromptBundle,
    LoopPromptBundleRef, LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, PromptMode,
};
use super::milestones::{
    LoopHostMilestoneEmitter, LoopHostMilestoneSink, PromptSkillContextMetadata,
};
use super::skill_context::skill_snippet_model_message_ref;

const DEFAULT_TEXT_ONLY_MESSAGE_LIMIT: usize = 32;
const MAX_TEXT_ONLY_MESSAGE_LIMIT: usize = 128;

type CurrentSurfaceVersionLookup =
    dyn Fn() -> Result<Option<CapabilitySurfaceVersion>, AgentLoopHostError> + Send + Sync;

/// Text-only host-managed prompt bundle port.
///
/// This adapter validates that prompt requests are scoped to the current
/// [`LoopRunContext`], loads bounded transcript context through a
/// [`LoopContextPort`], returns model-message references, and emits a
/// `prompt_bundle_built` milestone containing only metadata. It currently
/// supports [`PromptMode::TextOnly`] only; checkpoint-backed prompt state and
/// memory snippet materialization fail closed until dedicated host stores are
/// wired. Instruction snippets are surfaced as host-owned system message refs.
#[derive(Clone)]
pub struct HostManagedLoopPromptPort<C, S>
where
    C: LoopContextPort + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    context: LoopRunContext,
    context_port: Arc<C>,
    milestones: LoopHostMilestoneEmitter<S>,
    default_message_limit: usize,
    current_surface_version: Option<Arc<CurrentSurfaceVersionLookup>>,
}

impl<C, S> HostManagedLoopPromptPort<C, S>
where
    C: LoopContextPort + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    pub fn new(context: LoopRunContext, context_port: Arc<C>, milestone_sink: Arc<S>) -> Self {
        Self {
            context: context.clone(),
            context_port,
            milestones: LoopHostMilestoneEmitter::new(context, milestone_sink),
            default_message_limit: DEFAULT_TEXT_ONLY_MESSAGE_LIMIT,
            current_surface_version: None,
        }
    }

    pub fn with_default_message_limit(mut self, default_message_limit: usize) -> Self {
        self.default_message_limit = default_message_limit.clamp(1, MAX_TEXT_ONLY_MESSAGE_LIMIT);
        self
    }

    pub fn with_current_surface_version(
        self,
        current_surface_version: CapabilitySurfaceVersion,
    ) -> Self {
        self.with_current_surface_version_lookup(move || Ok(Some(current_surface_version.clone())))
    }

    pub fn with_current_surface_version_lookup<F>(mut self, lookup: F) -> Self
    where
        F: Fn() -> Result<Option<CapabilitySurfaceVersion>, AgentLoopHostError>
            + Send
            + Sync
            + 'static,
    {
        self.current_surface_version = Some(Arc::new(lookup));
        self
    }

    fn validate_request(
        &self,
        request: &LoopPromptBundleRequest,
    ) -> Result<(), AgentLoopHostError> {
        if request.mode != PromptMode::TextOnly {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                "prompt mode is not supported by the text-only prompt port",
            ));
        }

        if request
            .context_cursor
            .as_ref()
            .is_some_and(|cursor| !cursor.is_for_run(&self.context))
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "prompt context cursor is not scoped to this loop run",
            ));
        }

        if let Some(surface_version) = request.surface_version.as_ref() {
            let Some(current_surface_version) = self.current_surface_version.as_ref() else {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "prompt surface version cannot be validated by this prompt port",
                ));
            };
            let Some(current_surface_version) = current_surface_version()? else {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "prompt surface version cannot be validated by this prompt port",
                ));
            };
            if surface_version != &current_surface_version {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::StaleSurface,
                    "prompt surface version is stale or unknown",
                ));
            }
        }

        if let Some(state_ref) = request.checkpoint_state_ref.as_ref() {
            let run_prefix = format!("checkpoint:{}:", self.context.run_id);
            if !state_ref.as_str().starts_with(&run_prefix) {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::ScopeMismatch,
                    "prompt checkpoint state ref is not scoped to this loop run",
                ));
            }
            if !state_ref.is_for_run(&self.context) {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::InvalidInvocation,
                    "prompt checkpoint state ref is malformed",
                ));
            }
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "checkpoint prompt state is not supported by the text-only prompt port",
            ));
        }

        if matches!(request.max_messages, Some(0)) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "prompt message limit must be greater than zero",
            ));
        }

        Ok(())
    }

    fn message_limit(&self, request: &LoopPromptBundleRequest) -> usize {
        request
            .max_messages
            .map(|messages| messages as usize)
            .unwrap_or(self.default_message_limit)
            .clamp(1, MAX_TEXT_ONLY_MESSAGE_LIMIT)
    }

    fn ensure_supported_context_shape(
        context: &LoopContextBundle,
    ) -> Result<(), AgentLoopHostError> {
        if !context.memory_snippets.is_empty() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                "text-only prompt port cannot materialize memory snippets",
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl<C, S> LoopPromptPort for HostManagedLoopPromptPort<C, S>
where
    C: LoopContextPort + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.validate_request(&request)?;
        let context = self
            .context_port
            .load_loop_context(LoopContextRequest {
                after: request.context_cursor.clone(),
                limit: self.message_limit(&request),
            })
            .await?;
        Self::ensure_supported_context_shape(&context)?;
        let mut messages = Vec::with_capacity(
            context.identity_messages.len()
                + context.instruction_snippets.len()
                + context.messages.len(),
        );
        messages.extend(
            context
                .identity_messages
                .into_iter()
                .map(context_message_to_model_message),
        );

        let mut skill_context = Vec::with_capacity(context.instruction_snippets.len());
        for (ordinal, snippet) in context.instruction_snippets.into_iter().enumerate() {
            let content_ref = skill_snippet_model_message_ref(
                &snippet.snippet_ref,
                &snippet.safe_summary,
                ordinal,
            )?;
            match snippet.metadata.as_ref() {
                Some(metadata) => skill_context.push(PromptSkillContextMetadata {
                    ordinal,
                    source_name: metadata.source_name.clone(),
                    trust_level: metadata.trust_level.clone(),
                }),
                None if snippet.snippet_ref.starts_with("skill:") => {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::Internal,
                        "skill instruction snippet metadata is missing",
                    ));
                }
                None => {}
            }
            messages.push(LoopModelMessage {
                role: "system".to_string(),
                content_ref,
            });
        }
        messages.extend(
            context
                .messages
                .into_iter()
                .map(context_message_to_model_message),
        );
        let bundle = LoopPromptBundle {
            bundle_ref: LoopPromptBundleRef::fresh_for_run(&self.context),
            messages,
            surface_version: request.surface_version.clone(),
        };
        self.milestones
            .prompt_bundle_built(
                bundle.bundle_ref.clone(),
                request.mode,
                bundle.surface_version.clone(),
                bundle.messages.len(),
                skill_context,
            )
            .await?;
        Ok(bundle)
    }
}

fn context_message_to_model_message(message: LoopContextMessage) -> LoopModelMessage {
    LoopModelMessage {
        role: message.role,
        content_ref: message.message_ref,
    }
}
