//! Document metadata and hygiene contract types.

use serde::{Deserialize, Serialize};

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
