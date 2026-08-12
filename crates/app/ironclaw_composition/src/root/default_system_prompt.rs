use std::{
    collections::HashMap,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use ironclaw_host_api::turn::TurnOriginKind;
use ironclaw_loop_contracts::{LoopRunContext, PromptMode};
// The prompt *content* is owned by the loop tier — `ironclaw_loop_host`, beside
// its other `prompts/*.md` assets and beside the `HostIdentityContextSource`
// this module implements (PROPOSAL §6.10.1). What stays here is assembly and
// the boot-time seeding of the user-editable `SYSTEM.md`, which is `std::fs`
// work on a real host path and belongs to the composition root.
use ironclaw_loop_host::{
    BENCHMARKING_MODE_PROTOCOL_PROMPT, DEFAULT_SYSTEM_PROMPT, HostIdentityContextBuildError,
    HostIdentityContextCandidate, HostIdentityContextSource, HostIdentityMessageContent,
    IdentityApplicability, IdentityFileName, SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT,
    SELF_KNOWLEDGE_PROTOCOL_PROMPT, TOOL_DISCLOSURE_PROTOCOL_PROMPT, identity_message_ref,
};
use ironclaw_turns::LoopMessageRef;

const DEFAULT_SYSTEM_PROMPT_NAME: &str = "SYSTEM.md";
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
    /// When true, the progressive tool-disclosure protocol is appended to the
    /// system prompt so the model is told to discover deferred tools via
    /// `tool_search`. Set from the resolved tool-disclosure mode at build time;
    /// off ⇒ the prompt carries the file plus the unconditional self-knowledge
    /// section, and nothing that references the bridge tools.
    disclosure_protocol_active: bool,
    /// When true, the benchmarking-mode protocol is appended, telling the
    /// model no human is available to answer clarifying questions. Set from
    /// the `BENCHMARKING_MODE` env var at build time (see `runtime.rs`); off
    /// by default, so normal product usage is unaffected.
    benchmarking_mode_active: bool,
    loaded_identity_content: Arc<RwLock<HashMap<LoopMessageRef, HostIdentityMessageContent>>>,
}

impl DefaultSystemPromptIdentitySource {
    pub(crate) fn try_new(
        storage_root: PathBuf,
        prompt_path: PathBuf,
        disclosure_protocol_active: bool,
        benchmarking_mode_active: bool,
    ) -> Result<Self, DefaultSystemPromptError> {
        read_default_system_prompt(&storage_root, &prompt_path)?;
        Ok(Self {
            storage_root,
            prompt_path,
            disclosure_protocol_active,
            benchmarking_mode_active,
            loaded_identity_content: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn prompt_content(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<String, DefaultSystemPromptError> {
        // Append in memory (not to the seeded, user-editable file) so these
        // sections are system invariants independent of user edits to SYSTEM.md
        // — and so existing installs get them, not just freshly seeded ones.
        let mut content = read_default_system_prompt(&self.storage_root, &self.prompt_path)?;
        append_section(&mut content, SELF_KNOWLEDGE_PROTOCOL_PROMPT);
        if self.disclosure_protocol_active {
            append_section(&mut content, TOOL_DISCLOSURE_PROTOCOL_PROMPT);
        }
        if self.benchmarking_mode_active {
            append_section(&mut content, BENCHMARKING_MODE_PROTOCOL_PROMPT);
        }
        if matches!(
            run_context
                .product_context
                .as_ref()
                .map(|context| context.origin),
            Some(TurnOriginKind::ScheduledTrigger)
        ) {
            append_section(&mut content, SCHEDULED_TRIGGER_MODE_PROTOCOL_PROMPT);
        }
        Ok(content)
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
            .write_all(DEFAULT_SYSTEM_PROMPT.as_bytes())
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

/// Append an embedded prompt section after `content`, separated by a blank line
/// so the markdown heading always starts its own block regardless of how the
/// user's file ends.
fn append_section(content: &mut String, section: &str) {
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push('\n');
    content.push_str(section);
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
            reason: "path is outside the standalone storage root".to_string(),
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
            reason: "canonical path escapes the standalone storage root".to_string(),
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
            reason: "parent is outside the standalone storage root".to_string(),
        });
    }
    let relative_parent =
        parent
            .strip_prefix(storage_root)
            .map_err(|_| DefaultSystemPromptError::InvalidFile {
                path: parent.to_path_buf(),
                reason: "parent is outside the standalone storage root".to_string(),
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
        run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        let content = self
            .prompt_content(run_context)
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
    use ironclaw_host_api::{
        ids::{TenantId, ThreadId, UserId},
        turn::{ProductTurnContext, TurnOriginKind, TurnOwner},
    };
    use ironclaw_loop_contracts::{
        InMemoryRunProfileResolver, LoopRunContext, RunProfileResolutionRequest, RunProfileResolver,
    };
    use ironclaw_turns::{TurnId, TurnRunId, TurnScope};

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

    async fn run_context_with_origin(origin: TurnOriginKind) -> LoopRunContext {
        test_run_context()
            .await
            .with_product_context(ProductTurnContext::new(
                origin,
                None,
                None,
                TurnOwner::Personal {
                    user: UserId::new("prompt-test-owner").expect("valid user id"),
                },
            ))
    }

    #[tokio::test]
    async fn default_system_prompt_loads_and_resolves_as_identity_message() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().canonicalize().expect("canonical root");
        let prompt_path = storage_root.join("system/prompts/default-system.md");
        seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
        let source = DefaultSystemPromptIdentitySource::try_new(
            storage_root,
            prompt_path.clone(),
            false,
            false,
        )
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
            "source should seed the editable standalone prompt file"
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
        // Self-knowledge must be grounded in the published docs site rather than
        // recalled from training data (#6734): the prompt has to name the
        // llms.txt index and the `.md` raw-markdown suffix, or the model has no
        // way to look its own capabilities up. The guidance is ground knowledge
        // about the runtime, so it is appended in memory rather than seeded into
        // the user-editable file — otherwise only fresh installs would get it.
        assert!(
            !std::fs::read_to_string(&prompt_path)
                .expect("seeded prompt reads")
                .contains("docs.ironclaw.com"),
            "docs grounding must not be seeded into the user-editable prompt file"
        );
        assert!(
            content
                .content
                .contains("https://docs.ironclaw.com/llms.txt"),
            "prompt must point capability questions at the docs index"
        );
        assert!(
            content.content.contains(".md"),
            "prompt must teach the raw-markdown `.md` suffix for docs pages"
        );
        assert!(
            !content.content.contains("tool_search"),
            "disclosure-off prompt must not mention the bridge tools"
        );
    }

    #[tokio::test]
    async fn disclosure_active_appends_tool_search_protocol_to_system_prompt() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().canonicalize().expect("canonical root");
        let prompt_path = storage_root.join("system/prompts/default-system.md");
        seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");

        let off = DefaultSystemPromptIdentitySource::try_new(
            storage_root.clone(),
            prompt_path.clone(),
            false,
            false,
        )
        .expect("off source loads");
        let on = DefaultSystemPromptIdentitySource::try_new(storage_root, prompt_path, true, false)
            .expect("on source loads");
        let context = test_run_context().await;

        async fn resolve_content(
            source: &DefaultSystemPromptIdentitySource,
            context: &LoopRunContext,
        ) -> String {
            let candidates = source
                .load_identity_candidates(context, PromptMode::TextOnly)
                .await
                .expect("candidates load");
            source
                .resolve_identity_message_content(
                    context,
                    candidates[0]
                        .message_ref
                        .as_ref()
                        .expect("trusted identity has ref"),
                )
                .await
                .expect("resolve content")
                .expect("content exists")
                .content
        }

        let off_content = resolve_content(&off, &context).await;
        let on_content = resolve_content(&on, &context).await;

        // The base prompt is preserved verbatim, and only the active source teaches
        // the search/describe/call protocol — so the model is actually told the
        // deferred long tail exists and how to reach it.
        assert!(on_content.starts_with(off_content.trim_end()));
        assert!(!off_content.contains("tool_search"));
        assert!(on_content.contains("tool_search"));
        assert!(on_content.contains("tool_describe"));
        assert!(on_content.contains("tool_call"));
        assert!(on_content.contains("Tool Discovery"));
        assert!(
            on_content.contains("When `tool_search` is present"),
            "bridged-mode guidance must be conditional on the outgoing surface actually advertising tool_search"
        );
        assert!(
            on_content.contains("When `tool_search` is absent"),
            "below-threshold guidance must direct the model to use the complete direct surface"
        );
    }

    #[tokio::test]
    async fn benchmarking_mode_active_appends_no_human_protocol_to_system_prompt() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().canonicalize().expect("canonical root");
        let prompt_path = storage_root.join("system/prompts/default-system.md");
        seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");

        let off = DefaultSystemPromptIdentitySource::try_new(
            storage_root.clone(),
            prompt_path.clone(),
            false,
            false,
        )
        .expect("off source loads");
        let on = DefaultSystemPromptIdentitySource::try_new(storage_root, prompt_path, false, true)
            .expect("on source loads");
        let context = test_run_context().await;

        async fn resolve_content(
            source: &DefaultSystemPromptIdentitySource,
            context: &LoopRunContext,
        ) -> String {
            let candidates = source
                .load_identity_candidates(context, PromptMode::TextOnly)
                .await
                .expect("candidates load");
            source
                .resolve_identity_message_content(
                    context,
                    candidates[0]
                        .message_ref
                        .as_ref()
                        .expect("trusted identity has ref"),
                )
                .await
                .expect("resolve content")
                .expect("content exists")
                .content
        }

        let off_content = resolve_content(&off, &context).await;
        let on_content = resolve_content(&on, &context).await;

        // The base prompt is preserved verbatim, and only the active source
        // adds the no-human protocol — real product usage (mode off) is
        // byte-identical to today's prompt.
        assert!(on_content.starts_with(off_content.trim_end()));
        assert!(!off_content.contains("Automated Evaluation Mode"));
        assert!(on_content.contains("Automated Evaluation Mode"));
        assert!(on_content.contains("no one to answer a clarifying question"));
    }

    #[tokio::test]
    async fn scheduled_trigger_origin_appends_unattended_protocol_only_to_triggered_runs() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().canonicalize().expect("canonical root");
        let prompt_path = storage_root.join("system/prompts/default-system.md");
        seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
        let source =
            DefaultSystemPromptIdentitySource::try_new(storage_root, prompt_path, false, false)
                .expect("prompt loads");
        let interactive_context = run_context_with_origin(TurnOriginKind::Inbound).await;
        let scheduled_context = run_context_with_origin(TurnOriginKind::ScheduledTrigger).await;

        async fn resolve_content(
            source: &DefaultSystemPromptIdentitySource,
            context: &LoopRunContext,
        ) -> String {
            let candidates = source
                .load_identity_candidates(context, PromptMode::TextOnly)
                .await
                .expect("candidates load");
            source
                .resolve_identity_message_content(
                    context,
                    candidates[0]
                        .message_ref
                        .as_ref()
                        .expect("trusted identity has ref"),
                )
                .await
                .expect("resolve content")
                .expect("content exists")
                .content
        }

        let interactive_content = resolve_content(&source, &interactive_context).await;
        let scheduled_content = resolve_content(&source, &scheduled_context).await;

        assert!(
            !interactive_content.contains("Unattended Scheduled Run"),
            "interactive runs must retain the ordinary ask-the-user escape valve"
        );
        assert!(scheduled_content.contains("Unattended Scheduled Run"));
        assert!(scheduled_content.contains("There is no human present"));
        assert!(scheduled_content.contains("Never end the run with a question"));
        assert!(scheduled_content.contains("final reply is the run's recorded output"));
    }

    #[tokio::test]
    async fn default_system_prompt_reloads_edited_prompt_for_new_candidates() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().canonicalize().expect("canonical root");
        let prompt_path = storage_root.join("system/prompts/default-system.md");
        seed_default_system_prompt(&storage_root, &prompt_path).expect("prompt seeds");
        let source = DefaultSystemPromptIdentitySource::try_new(
            storage_root.clone(),
            prompt_path.clone(),
            false,
            false,
        )
        .expect("prompt loads");
        let context = test_run_context().await;
        let first_candidates = source
            .load_identity_candidates(&context, PromptMode::TextOnly)
            .await
            .expect("first candidates load");

        std::fs::write(&prompt_path, "edited standalone prompt").expect("prompt edits");
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

        // The user's edited base is preserved verbatim and stays first, but the
        // docs-grounding self-knowledge section is ground knowledge about the
        // runtime (#6734): it is appended unconditionally, so an install whose
        // SYSTEM.md predates the guidance (or was edited to drop it) still tells
        // the model to look its own capabilities up instead of guessing.
        assert!(content.content.starts_with("edited standalone prompt"));
        assert!(
            content.content.contains("## Self-Knowledge"),
            "self-knowledge guidance must be appended even when SYSTEM.md omits it"
        );
        assert!(
            content
                .content
                .contains("https://docs.ironclaw.com/llms.txt"),
            "appended guidance must point capability questions at the docs index"
        );
        assert!(
            content.content.contains(".md"),
            "appended guidance must teach the raw-markdown `.md` suffix for docs pages"
        );
        assert!(
            !content.content.contains("tool_search"),
            "disclosure-off prompt must not mention the bridge tools"
        );
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
