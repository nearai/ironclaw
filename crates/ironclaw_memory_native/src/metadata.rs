//! Document metadata, hygiene, write options, and `.config` inheritance.

use std::collections::HashMap;

use ironclaw_filesystem::FilesystemError;

use crate::path::MemoryDocumentPath;
use crate::repo::MemoryDocumentRepository;

pub use ironclaw_memory::{CONFIG_FILE_NAME, DocumentMetadata, HygieneMetadata};

/// Options resolved by the memory backend before persisting a document write.
#[derive(Debug, Clone, Default)]
pub struct MemoryWriteOptions {
    pub metadata: DocumentMetadata,
    pub changed_by: Option<String>,
}

impl MemoryWriteOptions {
    pub fn set_changed_by(mut self, changed_by: impl Into<String>) -> Self {
        self.changed_by = Some(changed_by.into());
        self
    }
}

/// Backend-facing options for a document write.
#[derive(Debug, Clone, Default)]
pub struct MemoryBackendWriteOptions {
    pub metadata_overlay: Option<DocumentMetadata>,
    /// Native-owned coordination flag, `pub(crate)` so it can be set **only**
    /// by this crate's filesystem adapter after it has already run prompt-write
    /// safety. A foreign crate can construct `MemoryBackendWriteOptions` only via
    /// `Default` (this field is not nameable outside the crate), so the bypass
    /// cannot be forged. The backend skips its own `enforce_prompt_write_safety`
    /// pass when this is true; it defaults to `false`, so any direct backend
    /// caller fails closed and the backend re-enforces (zmanian #3180 HIGH
    /// protected prompt files such as `SOUL.md` / `BOOTSTRAP.md`).
    pub(crate) prompt_safety_already_enforced: bool,
}

impl MemoryBackendWriteOptions {
    /// Backend write options carrying a metadata overlay. Prompt-safety
    /// enforcement is left at the fail-closed default — only this crate's
    /// filesystem adapter can set `prompt_safety_already_enforced`, so foreign
    /// callers construct options through this (or `Default`) and cannot forge
    /// the bypass.
    pub fn with_metadata_overlay(metadata_overlay: Option<DocumentMetadata>) -> Self {
        Self {
            metadata_overlay,
            prompt_safety_already_enforced: false,
        }
    }

    pub(crate) fn with_prompt_safety_already_enforced(mut self) -> Self {
        self.prompt_safety_already_enforced = true;
        self
    }
}

pub(crate) async fn resolve_document_metadata<R>(
    repository: &R,
    path: &MemoryDocumentPath,
) -> Result<DocumentMetadata, FilesystemError>
where
    R: MemoryDocumentRepository + ?Sized,
{
    let doc_meta = repository
        .read_document_metadata(path)
        .await?
        .unwrap_or_else(|| serde_json::json!({}));
    let configs = repository.list_documents(path.scope()).await?;
    let mut config_metadata = HashMap::<String, serde_json::Value>::new();
    for config_path in configs
        .into_iter()
        .filter(|candidate| is_config_path(candidate.relative_path()))
    {
        if let Some(metadata) = repository.read_document_metadata(&config_path).await? {
            config_metadata.insert(config_path.relative_path().to_string(), metadata);
        }
    }
    let base = find_nearest_config(path.relative_path(), &config_metadata)
        .unwrap_or_else(|| serde_json::json!({}));
    Ok(DocumentMetadata::from_value(&DocumentMetadata::merge(
        &base, &doc_meta,
    )))
}

pub(crate) fn is_config_path(path: &str) -> bool {
    path.rsplit('/').next().unwrap_or(path) == CONFIG_FILE_NAME
}

pub(crate) fn find_nearest_config(
    path: &str,
    configs: &HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut current = path;
    while let Some(slash_pos) = current.rfind('/') {
        let parent = current.get(..slash_pos)?;
        let config_path = format!("{parent}/{CONFIG_FILE_NAME}");
        if let Some(metadata) = configs.get(config_path.as_str()) {
            return Some(metadata.clone());
        }
        current = parent;
    }
    configs.get(CONFIG_FILE_NAME).cloned()
}
