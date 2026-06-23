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

/// Backend-facing options for a document write.
#[derive(Debug, Clone, Default)]
pub struct MemoryBackendWriteOptions {
    pub metadata_overlay: Option<DocumentMetadata>,
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
