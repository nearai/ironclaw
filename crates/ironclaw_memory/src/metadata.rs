//! Document metadata, hygiene, write options, and `.config` inheritance.

use std::collections::HashMap;

use ironclaw_filesystem::FilesystemError;
use serde::{Deserialize, Serialize};

use crate::path::MemoryDocumentPath;
use crate::repo::MemoryDocumentRepository;

/// Name of the folder-level configuration document.
pub const CONFIG_FILE_NAME: &str = ".config";

/// Typed overlay for memory document metadata.
///
/// Ported from the current workspace metadata model. Unknown fields are
/// preserved for forward compatibility.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DocumentMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_indexing: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_versioning: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hygiene: Option<HygieneMetadata>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl DocumentMetadata {
    pub fn from_value(value: &serde_json::Value) -> Self {
        match serde_json::from_value(value.clone()) {
            Ok(metadata) => metadata,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    raw = %value,
                    "failed to deserialize DocumentMetadata; falling back to defaults"
                );
                Self::default()
            }
        }
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }

    pub fn merge(base: &serde_json::Value, overlay: &serde_json::Value) -> serde_json::Value {
        let mut merged = match base {
            serde_json::Value::Object(map) => map.clone(),
            _ => serde_json::Map::new(),
        };
        if let serde_json::Value::Object(over) = overlay {
            for (key, value) in over {
                merged.insert(key.clone(), value.clone());
            }
        }
        serde_json::Value::Object(merged)
    }
}

/// Hygiene metadata preserved from the current workspace metadata model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HygieneMetadata {
    pub enabled: bool,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_retention_days() -> u32 {
    30
}

/// Options resolved by the memory backend before persisting a document write.
#[derive(Debug, Clone, Default)]
pub struct MemoryWriteOptions {
    pub metadata: DocumentMetadata,
    pub changed_by: Option<String>,
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
