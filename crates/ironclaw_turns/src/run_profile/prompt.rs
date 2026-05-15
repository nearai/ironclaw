use std::sync::Arc;

use async_trait::async_trait;

use super::host::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilitySurfaceVersion, LoopContextPort,
    LoopContextRequest, LoopPromptBundle, LoopPromptBundleAuthority, LoopPromptBundleRef,
    LoopPromptBundleRequest, LoopPromptPort, LoopRunContext, PromptMode, VisibleCapabilitySurface,
};
use super::instruction_bundle::{
    InstructionBundleBuilder, InstructionBundleRequest, InstructionMaterializationStore,
    InstructionSafetyContext,
};
use super::milestones::LoopHostMilestoneEmitter;
use super::milestones::LoopHostMilestoneSink;

const DEFAULT_TEXT_ONLY_MESSAGE_LIMIT: usize = 32;
const MAX_TEXT_ONLY_MESSAGE_LIMIT: usize = 128;

type CurrentSurfaceVersionLookup =
    dyn Fn() -> Result<Option<CapabilitySurfaceVersion>, AgentLoopHostError> + Send + Sync;
type CurrentSurfaceLookup =
    dyn Fn() -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError> + Send + Sync;

/// Text-only host-managed prompt bundle port.
///
/// This adapter validates that prompt requests are scoped to the current
/// [`LoopRunContext`], loads bounded transcript context through a
/// [`LoopContextPort`], returns model-message references, and emits a
/// `prompt_bundle_built` milestone containing only metadata. It currently
/// supports [`PromptMode::TextOnly`] only; checkpoint-backed prompt state fails
/// closed until dedicated host stores are wired. Instruction and memory snippets
/// are surfaced as host-owned system message refs.
#[derive(Clone)]
pub struct HostManagedLoopPromptPort<C, S>
where
    C: LoopContextPort + ?Sized,
    S: LoopHostMilestoneSink + ?Sized,
{
    context: LoopRunContext,
    context_port: Arc<C>,
    milestones: LoopHostMilestoneEmitter<S>,
    prompt_authority: LoopPromptBundleAuthority,
    default_message_limit: usize,
    current_surface_version: Option<Arc<CurrentSurfaceVersionLookup>>,
    current_surface: Option<Arc<CurrentSurfaceLookup>>,
    safety_context: Option<InstructionSafetyContext>,
    instruction_materialization_store: Option<Arc<dyn InstructionMaterializationStore>>,
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
            prompt_authority: LoopPromptBundleAuthority::shared(),
            default_message_limit: DEFAULT_TEXT_ONLY_MESSAGE_LIMIT,
            current_surface_version: None,
            current_surface: None,
            safety_context: None,
            instruction_materialization_store: None,
        }
    }

    pub fn with_default_message_limit(mut self, default_message_limit: usize) -> Self {
        self.default_message_limit = default_message_limit.clamp(1, MAX_TEXT_ONLY_MESSAGE_LIMIT);
        self
    }

    pub fn with_prompt_bundle_authority(
        mut self,
        prompt_authority: LoopPromptBundleAuthority,
    ) -> Self {
        self.prompt_authority = prompt_authority;
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

    pub fn with_current_surface(mut self, current_surface: VisibleCapabilitySurface) -> Self {
        let current_surface_for_version = current_surface.clone();
        self.current_surface_version = Some(Arc::new(move || {
            Ok(Some(current_surface_for_version.version.clone()))
        }));
        self.current_surface = Some(Arc::new(move || Ok(Some(current_surface.clone()))));
        self
    }

    pub fn with_current_surface_lookup<F>(mut self, lookup: F) -> Self
    where
        F: Fn() -> Result<Option<VisibleCapabilitySurface>, AgentLoopHostError>
            + Send
            + Sync
            + 'static,
    {
        let lookup = Arc::new(lookup);
        let lookup_for_version = Arc::clone(&lookup);
        self.current_surface_version = Some(Arc::new(move || {
            Ok(lookup_for_version()?.map(|surface| surface.version))
        }));
        self.current_surface = Some(lookup);
        self
    }

    pub fn with_safety_context(mut self, safety_context: InstructionSafetyContext) -> Self {
        self.safety_context = Some(safety_context);
        self
    }

    pub fn with_instruction_materialization_store(
        mut self,
        store: Arc<dyn InstructionMaterializationStore>,
    ) -> Self {
        self.instruction_materialization_store = Some(store);
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

        if !request.inline_messages.is_empty() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::PolicyDenied,
                "inline_messages not yet supported by this prompt builder",
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

    fn instruction_builder(&self) -> InstructionBundleBuilder {
        InstructionBundleBuilder::new(self.context.clone())
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
        let visible_surface = if request.surface_version.is_some() {
            match self.current_surface.as_ref() {
                Some(current_surface) => current_surface()?,
                None => None,
            }
        } else {
            None
        };
        let instruction_bundle = self.instruction_builder().build(InstructionBundleRequest {
            context_bundle: context,
            visible_surface,
            safety_context: self.safety_context.clone(),
        })?;
        if let Some(store) = self.instruction_materialization_store.as_ref() {
            store.put_materialized_messages(
                &self.context,
                instruction_bundle.materialized_messages.clone(),
            )?;
        } else if instruction_bundle.requires_materialization_store {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "instruction materialization store is required for this prompt bundle",
            ));
        }
        let bundle = LoopPromptBundle {
            bundle_ref: LoopPromptBundleRef::fresh_for_run(&self.context),
            messages: instruction_bundle.messages,
            surface_version: request.surface_version.clone(),
            instruction_fingerprint: Some(instruction_bundle.fingerprint),
        };
        self.prompt_authority.issue_bundle(&self.context, &bundle)?;
        self.milestones
            .prompt_bundle_built(
                bundle.bundle_ref.clone(),
                request.mode,
                bundle.surface_version.clone(),
                bundle.messages.len(),
                instruction_bundle.skill_context,
            )
            .await?;
        Ok(bundle)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};

    use super::*;
    use crate::{
        RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
        run_profile::{
            InMemoryInstructionMaterializationStore, InMemoryLoopHostMilestoneSink,
            LoopContextBundle, LoopContextMessage, LoopInlineMessage, LoopInlineMessageRole,
            LoopSafeSummary, ResolvedRunProfile,
        },
    };

    struct PanicContextPort;

    #[async_trait]
    impl LoopContextPort for PanicContextPort {
        async fn load_loop_context(
            &self,
            _request: LoopContextRequest,
        ) -> Result<LoopContextBundle, AgentLoopHostError> {
            panic!("inline message guard should run before context loading")
        }
    }

    #[tokio::test]
    async fn host_managed_prompt_port_rejects_inline_messages() {
        let context = test_context();
        let port = HostManagedLoopPromptPort::new(
            context,
            Arc::new(PanicContextPort),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
        );

        let error = port
            .build_prompt_bundle(LoopPromptBundleRequest {
                mode: PromptMode::TextOnly,
                context_cursor: None,
                surface_version: None,
                checkpoint_state_ref: None,
                max_messages: Some(8),
                inline_messages: vec![LoopInlineMessage {
                    role: LoopInlineMessageRole::User,
                    safe_body: LoopSafeSummary::new("safe inline nudge").unwrap(),
                }],
            })
            .await
            .unwrap_err();

        assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
        assert_eq!(
            error.safe_summary,
            "inline_messages not yet supported by this prompt builder"
        );
    }

    /// A context port that returns configurable identity and body messages.
    struct StubContextPort {
        identity_messages: Vec<LoopContextMessage>,
        messages: Vec<LoopContextMessage>,
    }

    impl StubContextPort {
        fn new(
            identity_messages: Vec<LoopContextMessage>,
            messages: Vec<LoopContextMessage>,
        ) -> Self {
            Self {
                identity_messages,
                messages,
            }
        }
    }

    #[async_trait]
    impl LoopContextPort for StubContextPort {
        async fn load_loop_context(
            &self,
            _request: LoopContextRequest,
        ) -> Result<LoopContextBundle, AgentLoopHostError> {
            Ok(LoopContextBundle {
                identity_messages: self.identity_messages.clone(),
                messages: self.messages.clone(),
                instruction_snippets: vec![],
                memory_snippets: vec![],
            })
        }
    }

    /// `LoopContextMessage { message_ref: None, ... }` is a summary-only entry.
    /// `HostManagedLoopPromptPort` must materialize a stable model message ref
    /// from `safe_summary` instead of silently dropping the entry.
    #[tokio::test]
    async fn host_managed_prompt_port_materializes_summary_only_identity_messages() {
        let context = test_context();
        let summary_text = "You are a helpful assistant acting on behalf of the user.";
        let identity_msg = LoopContextMessage {
            message_ref: None,
            role: "system".to_string(),
            safe_summary: summary_text.to_string(),
        };
        let store = Arc::new(InMemoryInstructionMaterializationStore::default());
        let port = HostManagedLoopPromptPort::new(
            context.clone(),
            Arc::new(StubContextPort::new(vec![identity_msg], vec![])),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
        )
        .with_instruction_materialization_store(store.clone());

        let bundle = port
            .build_prompt_bundle(LoopPromptBundleRequest {
                mode: PromptMode::TextOnly,
                context_cursor: None,
                surface_version: None,
                checkpoint_state_ref: None,
                max_messages: Some(8),
                inline_messages: vec![],
            })
            .await
            .expect("bundle should succeed for summary-only identity message");

        assert_eq!(
            bundle.messages.len(),
            1,
            "summary-only identity message must appear in the bundle (not be dropped)"
        );
        let msg = &bundle.messages[0];
        assert_eq!(msg.role, "system");
        assert!(
            msg.content_ref
                .as_str()
                .starts_with("msg:identity-summary."),
            "summary-only identity ref must use the msg:identity-summary. prefix, got: {}",
            msg.content_ref.as_str()
        );
        let materialized = store
            .get_materialized_message(&context, &msg.content_ref)
            .unwrap()
            .expect("summary-only identity message should be materialized");
        assert_eq!(materialized.safe_content, summary_text);
    }

    /// A summary-only entry in the main messages list (not just identity_messages)
    /// must also be materialized, not dropped.
    #[tokio::test]
    async fn host_managed_prompt_port_materializes_summary_only_body_messages() {
        let context = test_context();
        let summary_only = LoopContextMessage {
            message_ref: None,
            role: "user".to_string(),
            safe_summary: "What is the capital of France?".to_string(),
        };
        let store = Arc::new(InMemoryInstructionMaterializationStore::default());
        let port = HostManagedLoopPromptPort::new(
            context.clone(),
            Arc::new(StubContextPort::new(vec![], vec![summary_only])),
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
        )
        .with_instruction_materialization_store(store.clone());

        let bundle = port
            .build_prompt_bundle(LoopPromptBundleRequest {
                mode: PromptMode::TextOnly,
                context_cursor: None,
                surface_version: None,
                checkpoint_state_ref: None,
                max_messages: Some(8),
                inline_messages: vec![],
            })
            .await
            .expect("bundle should succeed for summary-only body message");

        assert_eq!(
            bundle.messages.len(),
            1,
            "summary-only body message must appear in the bundle (not be dropped)"
        );
        assert!(
            bundle.messages[0]
                .content_ref
                .as_str()
                .starts_with("msg:context-summary."),
            "summary-only ref must use the msg:context-summary. prefix"
        );
        let materialized = store
            .get_materialized_message(&context, &bundle.messages[0].content_ref)
            .unwrap()
            .expect("summary-only body message should be materialized");
        assert_eq!(materialized.safe_content, "What is the capital of France?");
    }

    fn test_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-prompt").unwrap(),
            Some(AgentId::new("agent-prompt").unwrap()),
            Some(ProjectId::new("project-prompt").unwrap()),
            ThreadId::new("thread-prompt").unwrap(),
        );
        let resolved_run_profile = ResolvedRunProfile::legacy_compatibility(
            RunProfileId::interactive_default(),
            RunProfileVersion::new(1),
            true,
        );
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }
}
