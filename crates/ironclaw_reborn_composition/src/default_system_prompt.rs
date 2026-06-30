use std::{
    collections::HashMap,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

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
const MAX_DEFAULT_SYSTEM_PROMPT_BYTES: u64 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DefaultSystemPromptError {
    #[error("default system prompt at {path} could not be initialized or read: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("default system prompt at {path} is invalid: {reason}")]
    InvalidFile { path: PathBuf, reason: String },
    #[error(
        "default system prompt at {path} is too large: {actual_bytes} bytes exceeds {max_bytes} bytes"
    )]
    TooLarge {
        path: PathBuf,
        actual_bytes: u64,
        max_bytes: u64,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct DefaultSystemPromptIdentitySource {
    storage_root: PathBuf,
    prompt_path: PathBuf,
    loaded_identity_content: Arc<RwLock<HashMap<LoopMessageRef, HostIdentityMessageContent>>>,
}

impl DefaultSystemPromptIdentitySource {
    pub(crate) fn try_new(
        storage_root: PathBuf,
        prompt_path: PathBuf,
    ) -> Result<Self, DefaultSystemPromptError> {
        read_default_system_prompt(&storage_root, &prompt_path)?;
        Ok(Self {
            storage_root,
            prompt_path,
            loaded_identity_content: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn prompt_content(&self) -> Result<String, DefaultSystemPromptError> {
        read_default_system_prompt(&self.storage_root, &self.prompt_path)
    }

    fn identity_name() -> Result<IdentityFileName, HostIdentityContextBuildError> {
        IdentityFileName::new(DEFAULT_SYSTEM_PROMPT_NAME)
    }

    fn message_ref_for(content: &str) -> Result<LoopMessageRef, HostIdentityContextBuildError> {
        let name = Self::identity_name()?;
        identity_message_ref(&name, content).map_err(|_| HostIdentityContextBuildError::Internal)
    }

    fn cache_identity_content(
        &self,
        message_ref: LoopMessageRef,
        content: String,
    ) -> Result<(), HostIdentityContextBuildError> {
        let name = Self::identity_name()?;
        self.loaded_identity_content
            .write()
            .map_err(|_| HostIdentityContextBuildError::Internal)?
            .insert(message_ref, HostIdentityMessageContent { name, content });
        Ok(())
    }
}

pub(crate) fn seed_default_system_prompt(
    storage_root: &Path,
    path: &Path,
) -> Result<(), DefaultSystemPromptError> {
    if path.symlink_metadata().is_ok() {
        validate_default_system_prompt(storage_root, path)?;
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        ensure_prompt_parent(storage_root, parent)?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file
            .write_all(DEFAULT_SYSTEM_PROMPT_EMBEDDED.as_bytes())
            .map_err(|source| DefaultSystemPromptError::Io {
                path: path.to_path_buf(),
                source,
            })?,
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            validate_default_system_prompt(storage_root, path)?;
        }
        Err(source) => {
            return Err(DefaultSystemPromptError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    validate_default_system_prompt(storage_root, path)?;
    Ok(())
}

fn read_default_system_prompt(
    storage_root: &Path,
    path: &Path,
) -> Result<String, DefaultSystemPromptError> {
    validate_default_system_prompt(storage_root, path)?;
    let content = std::fs::read_to_string(path).map_err(|source| DefaultSystemPromptError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if content.len() as u64 > MAX_DEFAULT_SYSTEM_PROMPT_BYTES {
        return Err(DefaultSystemPromptError::TooLarge {
            path: path.to_path_buf(),
            actual_bytes: content.len() as u64,
            max_bytes: MAX_DEFAULT_SYSTEM_PROMPT_BYTES,
        });
    }
    Ok(content)
}

fn validate_default_system_prompt(
    storage_root: &Path,
    path: &Path,
) -> Result<(), DefaultSystemPromptError> {
    if !path.starts_with(storage_root) {
        return Err(DefaultSystemPromptError::InvalidFile {
            path: path.to_path_buf(),
            reason: "path is outside the local-dev storage root".to_string(),
        });
    }
    let metadata = path
        .symlink_metadata()
        .map_err(|source| DefaultSystemPromptError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DefaultSystemPromptError::InvalidFile {
            path: path.to_path_buf(),
            reason: "path must be a regular file and must not be a symlink".to_string(),
        });
    }
    let canonical_root =
        storage_root
            .canonicalize()
            .map_err(|source| DefaultSystemPromptError::Io {
                path: storage_root.to_path_buf(),
                source,
            })?;
    let canonical_path = path
        .canonicalize()
        .map_err(|source| DefaultSystemPromptError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(DefaultSystemPromptError::InvalidFile {
            path: path.to_path_buf(),
            reason: "canonical path escapes the local-dev storage root".to_string(),
        });
    }
    if metadata.len() > MAX_DEFAULT_SYSTEM_PROMPT_BYTES {
        return Err(DefaultSystemPromptError::TooLarge {
            path: path.to_path_buf(),
            actual_bytes: metadata.len(),
            max_bytes: MAX_DEFAULT_SYSTEM_PROMPT_BYTES,
        });
    }
    Ok(())
}

fn ensure_prompt_parent(
    storage_root: &Path,
    parent: &Path,
) -> Result<(), DefaultSystemPromptError> {
    if !parent.starts_with(storage_root) {
        return Err(DefaultSystemPromptError::InvalidFile {
            path: parent.to_path_buf(),
            reason: "parent is outside the local-dev storage root".to_string(),
        });
    }
    let relative_parent =
        parent
            .strip_prefix(storage_root)
            .map_err(|_| DefaultSystemPromptError::InvalidFile {
                path: parent.to_path_buf(),
                reason: "parent is outside the local-dev storage root".to_string(),
            })?;
    let mut current = storage_root.to_path_buf();
    for component in relative_parent.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(DefaultSystemPromptError::InvalidFile {
                path: parent.to_path_buf(),
                reason: "parent contains an invalid path component".to_string(),
            });
        };
        current.push(part);
        match current.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(DefaultSystemPromptError::InvalidFile {
                    path: current,
                    reason: "parent components must be directories and must not be symlinks"
                        .to_string(),
                });
            }
            Ok(_) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|source| DefaultSystemPromptError::Io {
                    path: current.clone(),
                    source,
                })?;
            }
            Err(source) => {
                return Err(DefaultSystemPromptError::Io {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

#[async_trait]
impl HostIdentityContextSource for DefaultSystemPromptIdentitySource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let content = self
            .prompt_content()
            .map_err(|_| HostIdentityContextBuildError::SourceUnavailable)?;
        let name = Self::identity_name()?;
        let message_ref = Self::message_ref_for(&content)?;
        let model_visible_bytes = content.len();
        self.cache_identity_content(message_ref.clone(), content)?;
        Ok(vec![HostIdentityContextCandidate::new_trusted(
            name,
            message_ref,
            format!("identity file {DEFAULT_SYSTEM_PROMPT_NAME} available"),
            IdentityApplicability::Always,
            model_visible_bytes,
        )])
    }

    async fn resolve_identity_message_content(
        &self,
        _run_context: &LoopRunContext,
        message_ref: &LoopMessageRef,
    ) -> Result<Option<HostIdentityMessageContent>, HostIdentityContextBuildError> {
        self.loaded_identity_content
            .read()
            .map_err(|_| HostIdentityContextBuildError::Internal)
            .map(|cache| cache.get(message_ref).cloned())
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
        let storage_root = root.path().canonicalize().expect("canonical root");
        let prompt_path = storage_root.join("system/prompts/default-system.md");
        seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
        let source = DefaultSystemPromptIdentitySource::try_new(storage_root, prompt_path.clone())
            .expect("prompt loads");
        let context = test_run_context().await;

        let candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .expect("load candidates");

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].name.as_str(), DEFAULT_SYSTEM_PROMPT_NAME);
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

        assert!(
            content
                .content
                .contains("When a tool result is partial, truncated, failed")
        );
    }

    #[tokio::test]
    async fn default_system_prompt_reloads_edited_prompt_for_new_candidates() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().canonicalize().expect("canonical root");
        let prompt_path = storage_root.join("system/prompts/default-system.md");
        seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
        let source =
            DefaultSystemPromptIdentitySource::try_new(storage_root.clone(), prompt_path.clone())
                .expect("prompt loads");
        let context = test_run_context().await;
        let first_candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .expect("first candidates load");

        std::fs::write(&prompt_path, "edited local-dev prompt").expect("prompt edits");
        let edited_candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .expect("edited candidates load");

        assert_ne!(
            first_candidates[0].message_ref,
            edited_candidates[0].message_ref
        );
        let content = source
            .resolve_identity_message_content(
                &context,
                edited_candidates[0]
                    .message_ref
                    .as_ref()
                    .expect("trusted identity has ref"),
            )
            .await
            .expect("resolve edited content")
            .expect("edited content exists");

        assert_eq!(content.content, "edited local-dev prompt");
    }

    #[cfg(unix)]
    #[test]
    fn default_system_prompt_rejects_symlink() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().canonicalize().expect("canonical root");
        let prompt_path = storage_root.join("system/prompts/default-system.md");
        std::fs::create_dir_all(prompt_path.parent().expect("parent")).expect("prompt parent");
        let target = storage_root.join("target.md");
        std::fs::write(&target, "linked prompt").expect("target prompt");
        std::os::unix::fs::symlink(&target, &prompt_path).expect("prompt symlink");

        let error = seed_default_system_prompt(&storage_root, &prompt_path)
            .expect_err("symlink should be rejected");

        assert!(error.to_string().contains("must not be a symlink"));
    }
}
