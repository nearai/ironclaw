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

/// Which conditional protocol sections this runtime appends to the resolved
/// system prompt.
///
/// Named fields rather than positional `bool`s: every one of these describes a
/// capability the prompt is allowed to claim, and a swapped pair would silently
/// tell the model about a surface it does not have.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SystemPromptProtocols {
    /// When true, the progressive tool-disclosure protocol is appended to the
    /// system prompt so the model is told to discover deferred tools via
    /// `tool_search`. Set from the resolved tool-disclosure mode at build time;
    /// off ⇒ the prompt carries the file plus the unconditional self-knowledge
    /// section, and nothing that references the bridge tools.
    pub(crate) disclosure: bool,
    /// When true, the benchmarking-mode protocol is appended, telling the
    /// model no human is available to answer clarifying questions. Set from
    /// the `BENCHMARKING_MODE` env var at build time (see `runtime.rs`); off
    /// by default, so normal product usage is unaffected.
    pub(crate) benchmarking_mode: bool,
    /// The bound memory provider's own guidance text, appended verbatim, or
    /// `None` when no provider is bound or the bound one ships none. Resolved
    /// at build time from the provider's `[memory].guidance_doc` (see
    /// `runtime.rs`).
    ///
    /// Content rather than a flag, because the text is not the host's to write.
    /// It names concrete `ironclaw.memory.*` tools and describes one provider's
    /// recall behavior, so the provider that implements them owns what it says
    /// — a search-first backend needs to tell the model something different
    /// from one that serves a standing document. A `Disabled` binding registers
    /// no package and the model's surface carries no memory tools at all, so
    /// `None` there is what keeps the prompt from claiming a surface the
    /// deployment does not have. Same reasoning as `disclosure`: guidance that
    /// names concrete tools must not outlive those tools.
    pub(crate) memory_guidance: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DefaultSystemPromptIdentitySource {
    storage_root: PathBuf,
    prompt_path: PathBuf,
    protocols: SystemPromptProtocols,
    loaded_identity_content: Arc<RwLock<HashMap<LoopMessageRef, HostIdentityMessageContent>>>,
}

impl DefaultSystemPromptIdentitySource {
    pub(crate) fn try_new(
        storage_root: PathBuf,
        prompt_path: PathBuf,
        protocols: SystemPromptProtocols,
    ) -> Result<Self, DefaultSystemPromptError> {
        read_default_system_prompt(&storage_root, &prompt_path)?;
        Ok(Self {
            storage_root,
            prompt_path,
            protocols,
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
        if let Some(guidance) = self.protocols.memory_guidance.as_deref() {
            append_section(&mut content, guidance);
        }
        if self.protocols.disclosure {
            append_section(&mut content, TOOL_DISCLOSURE_PROTOCOL_PROMPT);
        }
        if self.protocols.benchmarking_mode {
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
mod tests;
