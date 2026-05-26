use std::{io, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostIdentityMessageContent, IdentityApplicability, IdentityFileName, identity_message_ref,
};
use ironclaw_turns::{
    LoopMessageRef,
    run_profile::{LoopRunContext, PromptMode},
};

const DEFAULT_SYSTEM_PROMPT_NAME: &str = "SYSTEM.md";
const DEFAULT_SYSTEM_PROMPT_EMBEDDED: &str = include_str!("../assets/prompts/default-system.md");

#[derive(Debug, thiserror::Error)]
pub(crate) enum DefaultSystemPromptError {
    #[error("default system prompt at {path} could not be initialized or read: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone)]
pub(crate) struct DefaultSystemPromptIdentitySource {
    content: Arc<str>,
}

impl DefaultSystemPromptIdentitySource {
    pub(crate) fn try_new(path: PathBuf) -> Result<Self, DefaultSystemPromptError> {
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(source) => return Err(DefaultSystemPromptError::Io { path, source }),
        };
        Ok(Self {
            content: Arc::from(content),
        })
    }

    fn prompt_content(&self) -> &str {
        &self.content
    }

    fn identity_name() -> Result<IdentityFileName, HostIdentityContextBuildError> {
        IdentityFileName::new(DEFAULT_SYSTEM_PROMPT_NAME)
    }

    fn message_ref_for(content: &str) -> Result<LoopMessageRef, HostIdentityContextBuildError> {
        let name = Self::identity_name()?;
        identity_message_ref(&name, content).map_err(|_| HostIdentityContextBuildError::Internal)
    }
}

pub(crate) fn seed_default_system_prompt(path: &PathBuf) -> Result<(), DefaultSystemPromptError> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| DefaultSystemPromptError::Io {
            path: path.clone(),
            source,
        })?;
    }
    std::fs::write(path, DEFAULT_SYSTEM_PROMPT_EMBEDDED).map_err(|source| {
        DefaultSystemPromptError::Io {
            path: path.clone(),
            source,
        }
    })
}

#[async_trait]
impl HostIdentityContextSource for DefaultSystemPromptIdentitySource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let name = Self::identity_name()?;
        let message_ref = Self::message_ref_for(self.prompt_content())?;
        Ok(vec![HostIdentityContextCandidate::new_trusted(
            name,
            message_ref,
            format!("identity file {DEFAULT_SYSTEM_PROMPT_NAME} available"),
            IdentityApplicability::Always,
            self.prompt_content().len(),
        )])
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        if message_ref != &Self::message_ref_for(self.prompt_content())? {
            return Ok(None);
        }
        Ok(Some(HostIdentityMessageContent {
            name: Self::identity_name()?,
            content: self.prompt_content().to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{TenantId, ThreadId};
    use ironclaw_turns::{
        RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId, TurnScope,
        run_profile::{InMemoryRunProfileResolver, LoopRunContext},
    };

    use super::*;

    async fn test_run_context() -> LoopRunContext {
        let profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("profile resolves");
        let scope = TurnScope::new(
            TenantId::new("tenant-default-system-prompt").expect("valid"),
            None,
            None,
            ThreadId::new("thread-default-system-prompt").expect("valid"),
        );
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), profile)
    }

    #[tokio::test]
    async fn default_system_prompt_loads_and_resolves_as_identity_message() {
        let root = tempfile::tempdir().expect("tempdir");
        let prompt_path = root.path().join("system/prompts/default-system.md");
        seed_default_system_prompt(&prompt_path).expect("prompt seeds");
        let source =
            DefaultSystemPromptIdentitySource::try_new(prompt_path.clone()).expect("prompt loads");
        let context = test_run_context().await;

        let candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .expect("load candidates");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name.as_str(), DEFAULT_SYSTEM_PROMPT_NAME);
        assert!(
            source
                .prompt_content()
                .contains("When a tool result is partial, truncated, failed")
        );
        assert!(
            prompt_path.exists(),
            "source should seed the editable local-dev prompt file"
        );

        let content = source
            .resolve_identity_message_content(
                &context,
                candidates[0]
                    .message_ref
                    .as_ref()
                    .expect("trusted identity has ref"),
            )
            .await
            .expect("resolve content")
            .expect("content exists");

        assert_eq!(content.content, source.prompt_content());
    }
}
