//! Memory document types for the workspace.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Well-known document paths.
///
/// These are conventional paths that have special meaning in the workspace.
/// Agents can create arbitrary paths beyond these.
pub mod paths {
    /// Long-term curated memory.
    pub const MEMORY: &str = "MEMORY.md";
    /// Agent identity (name, nature, vibe).
    pub const IDENTITY: &str = "IDENTITY.md";
    /// Core values and principles.
    pub const SOUL: &str = "SOUL.md";
    /// Behavior instructions.
    pub const AGENTS: &str = "AGENTS.md";
    /// User context (name, preferences).
    pub const USER: &str = "USER.md";
    /// Periodic checklist for heartbeat.
    pub const HEARTBEAT: &str = "HEARTBEAT.md";
    /// Root runbook/readme.
    pub const README: &str = "README.md";
    /// Daily logs directory.
    pub const DAILY_DIR: &str = "daily/";
    /// Context directory (for identity-related docs).
    pub const CONTEXT_DIR: &str = "context/";
    /// User-editable notes for environment-specific tool guidance.
    pub const TOOLS: &str = "TOOLS.md";
    /// First-run ritual file; self-deletes after onboarding completes.
    pub const BOOTSTRAP: &str = "BOOTSTRAP.md";
    /// User psychographic profile (JSON).
    pub const PROFILE: &str = "context/profile.json";
    /// Assistant behavioral directives (derived from profile).
    pub const ASSISTANT_DIRECTIVES: &str = "context/assistant-directives.md";
}

/// Well-known system paths for internal state.
///
/// Everything machine-managed lives under `.system/` — settings, extension
/// state, skill state, and v2 engine state (knowledge, projects, missions,
/// runtime threads/steps/events). The dot-prefix follows the Unix convention
/// for hidden internal state and signals "do not edit by hand".
///
/// Documents under `.system/` are excluded from search results via folder
/// `.config` metadata (`skip_indexing: true`) and are never auto-cleaned by
/// hygiene. They ARE versioned for audit trail.
pub mod system_paths {
    /// Root prefix for all machine-managed system state.
    #[allow(dead_code)] // Documents the convention; consumed via subdirectory constants
    pub const SYSTEM_PREFIX: &str = ".system/";
    /// Settings documents directory.
    pub const SETTINGS_PREFIX: &str = ".system/settings/";
    /// Extension state directory.
    pub const EXTENSIONS_PREFIX: &str = ".system/extensions/";
    /// Skill state directory.
    pub const SKILLS_PREFIX: &str = ".system/skills/";
    /// v2 engine state root. The bridge `store_adapter` defines its own
    /// per-subdirectory constants under this prefix; this constant exists
    /// as the canonical declaration of the convention.
    #[allow(dead_code)]
    pub const ENGINE_PREFIX: &str = ".system/engine/";
}

/// Name of the folder-level configuration document.
///
/// A document at `{directory}/.config` carries metadata flags that apply
/// as defaults to all documents in that directory (e.g., `skip_indexing`,
/// `hygiene` settings). Individual document metadata overrides folder defaults.
pub const CONFIG_FILE_NAME: &str = ".config";

/// Typed overlay for the `metadata` JSON field on [`MemoryDocument`].
///
/// Fields use `Option` so that only explicitly set flags participate in
/// the merge chain (document metadata → folder `.config` → system defaults).
/// Unknown fields are preserved via `serde(flatten)`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DocumentMetadata {
    /// When `true`, skip chunking and embedding for this document/folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_indexing: Option<bool>,

    /// When `true`, skip automatic versioning for this document/folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_versioning: Option<bool>,

    /// Hygiene (auto-cleanup) configuration for this folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hygiene: Option<HygieneMetadata>,

    /// Optional JSON Schema for content validation.
    ///
    /// When set, workspace write operations parse content as JSON and validate
    /// against this schema before persisting. Inherited via the `.config` chain
    /// (folder `.config` → document metadata), so a folder-level schema applies
    /// to all documents in that directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,

    /// Preserve unknown fields for forward compatibility.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl DocumentMetadata {
    /// Parse from a raw JSON [`serde_json::Value`].
    ///
    /// Returns [`Default`] if the value is not an object or cannot be parsed.
    pub fn from_value(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    /// Convert to a JSON [`serde_json::Value`].
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::json!({}))
    }

    /// Merge two metadata values: `overlay` keys win over `base` keys.
    ///
    /// This is a shallow merge at the top-level keys — nested objects are
    /// replaced wholesale, not recursively merged. This keeps the semantics
    /// simple and predictable across both PostgreSQL and libSQL.
    pub fn merge(base: &serde_json::Value, overlay: &serde_json::Value) -> serde_json::Value {
        let mut merged = match base {
            serde_json::Value::Object(map) => map.clone(),
            _ => serde_json::Map::new(),
        };
        if let serde_json::Value::Object(over) = overlay {
            for (k, v) in over {
                merged.insert(k.clone(), v.clone());
            }
        }
        serde_json::Value::Object(merged)
    }
}

/// Minimum allowed `retention_days` to prevent accidental mass-deletion.
const MIN_RETENTION_DAYS: u32 = 1;

/// Hygiene (auto-cleanup) settings for a folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HygieneMetadata {
    /// Whether this folder is a hygiene target.
    pub enabled: bool,

    /// Delete documents older than this many days (minimum: 1).
    #[serde(
        default = "default_retention_days",
        deserialize_with = "deserialize_retention_days"
    )]
    pub retention_days: u32,
}

fn default_retention_days() -> u32 {
    30
}

fn deserialize_retention_days<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = u32::deserialize(deserializer)?;
    Ok(value.max(MIN_RETENTION_DAYS))
}

/// A historical version of a workspace document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVersion {
    /// Version record ID.
    pub id: Uuid,
    /// Parent document ID.
    pub document_id: Uuid,
    /// Version number (1-based, monotonically increasing per document).
    pub version: i32,
    /// Full document content at this version.
    pub content: String,
    /// SHA-256 hash of `content` (hex-encoded, prefixed with `sha256:`).
    pub content_hash: String,
    /// When this version was created.
    pub created_at: DateTime<Utc>,
    /// Who/what created this version (e.g. `"agent"`, `"user:alice"`).
    pub changed_by: Option<String>,
}

/// Summary of a document version (without full content).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionSummary {
    /// Version number.
    pub version: i32,
    /// SHA-256 hash of the version's content.
    pub content_hash: String,
    /// When this version was created.
    pub created_at: DateTime<Utc>,
    /// Who/what created this version.
    pub changed_by: Option<String>,
}

/// Result of a workspace patch operation.
#[derive(Debug, Clone)]
pub struct PatchResult {
    /// The updated document.
    pub document: MemoryDocument,
    /// Number of replacements made.
    pub replacements: usize,
}

/// Compute a SHA-256 hash of content, returned as `"sha256:{hex}"`.
pub fn content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{:x}", result)
}

/// Check if a path refers to a `.config` document.
pub fn is_config_path(path: &str) -> bool {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    file_name == CONFIG_FILE_NAME
}

/// Paths treated as identity documents for multi-scope isolation.
///
/// These files are always read from the primary scope only — never from
/// secondary read scopes. This prevents silent identity inheritance
/// (e.g., user A accidentally presenting as user B).
pub const IDENTITY_PATHS: &[&str] = &[
    paths::IDENTITY,
    paths::SOUL,
    paths::AGENTS,
    paths::USER,
    paths::TOOLS,
    paths::BOOTSTRAP,
];

/// Check if a path is an identity document that must be isolated to primary scope.
pub fn is_identity_path(path: &str) -> bool {
    IDENTITY_PATHS.contains(&path)
}

/// A memory document stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDocument {
    /// Unique document ID.
    pub id: Uuid,
    /// User identifier.
    pub user_id: String,
    /// Optional agent ID for multi-agent isolation.
    pub agent_id: Option<Uuid>,
    /// File path within the workspace (e.g., "context/vision.md").
    pub path: String,
    /// Full document content.
    pub content: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Flexible metadata.
    pub metadata: serde_json::Value,
}

impl MemoryDocument {
    /// Create a new document with a path.
    pub fn new(
        user_id: impl Into<String>,
        agent_id: Option<Uuid>,
        path: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id: user_id.into(),
            agent_id,
            path: path.into(),
            content: String::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Object(serde_json::Map::new()),
        }
    }

    /// Get the file name from the path.
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// Get the parent directory from the path.
    pub fn parent_dir(&self) -> Option<&str> {
        let idx = self.path.rfind('/')?;
        Some(&self.path[..idx])
    }

    /// Check if the document is empty.
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Get word count.
    pub fn word_count(&self) -> usize {
        self.content.split_whitespace().count()
    }

    /// Check if this is a well-known identity document.
    pub fn is_identity_document(&self) -> bool {
        is_identity_path(&self.path)
    }
}

/// An entry in a workspace directory listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    /// Path relative to listing directory.
    pub path: String,
    /// True if this is a directory (has children).
    pub is_directory: bool,
    /// Last update timestamp (latest among children for directories).
    pub updated_at: Option<DateTime<Utc>>,
    /// Preview of content (first ~200 chars, None for directories).
    pub content_preview: Option<String>,
}

impl WorkspaceEntry {
    /// Get the entry name (last path component).
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// Merge workspace entries from multiple scopes into a deduplicated, sorted list.
///
/// When the same path appears in multiple scopes:
/// - Keeps the most recent `updated_at`
/// - If any scope marks it as a directory, the merged entry is a directory
pub fn merge_workspace_entries(
    entries: impl IntoIterator<Item = WorkspaceEntry>,
) -> Vec<WorkspaceEntry> {
    let mut seen = std::collections::HashMap::new();
    for entry in entries {
        seen.entry(entry.path.clone())
            .and_modify(|existing: &mut WorkspaceEntry| {
                // Keep the most recent updated_at (and its content_preview)
                if let (Some(existing_ts), Some(new_ts)) = (&existing.updated_at, &entry.updated_at)
                {
                    if new_ts > existing_ts {
                        existing.updated_at = Some(*new_ts);
                        existing.content_preview = entry.content_preview.clone();
                    }
                } else if existing.updated_at.is_none() {
                    existing.updated_at = entry.updated_at;
                    existing.content_preview = entry.content_preview.clone();
                }
                // If either is a directory, mark as directory
                if entry.is_directory {
                    existing.is_directory = true;
                    existing.content_preview = None;
                }
            })
            .or_insert(entry);
    }
    let mut result: Vec<WorkspaceEntry> = seen.into_values().collect();
    result.sort_by(|a, b| a.path.cmp(&b.path));
    result
}

/// A chunk of a memory document for search indexing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryChunk {
    /// Unique chunk ID.
    pub id: Uuid,
    /// Parent document ID.
    pub document_id: Uuid,
    /// Position in the document (0-based).
    pub chunk_index: i32,
    /// Chunk text content.
    pub content: String,
    /// Embedding vector (if generated).
    pub embedding: Option<Vec<f32>>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl MemoryChunk {
    /// Create a new chunk (not persisted yet).
    pub fn new(document_id: Uuid, chunk_index: i32, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            document_id,
            chunk_index,
            content: content.into(),
            embedding: None,
            created_at: Utc::now(),
        }
    }

    /// Set the embedding.
    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_document_new() {
        let doc = MemoryDocument::new("user1", None, "context/vision.md");
        assert_eq!(doc.user_id, "user1");
        assert_eq!(doc.path, "context/vision.md");
        assert!(doc.content.is_empty());
    }

    #[test]
    fn test_memory_document_file_name() {
        let doc = MemoryDocument::new("user1", None, "projects/alpha/README.md");
        assert_eq!(doc.file_name(), "README.md");
    }

    #[test]
    fn test_memory_document_parent_dir() {
        let doc = MemoryDocument::new("user1", None, "projects/alpha/README.md");
        assert_eq!(doc.parent_dir(), Some("projects/alpha"));

        let root_doc = MemoryDocument::new("user1", None, "README.md");
        assert_eq!(root_doc.parent_dir(), None);
    }

    #[test]
    fn test_memory_document_word_count() {
        let mut doc = MemoryDocument::new("user1", None, "MEMORY.md");
        assert_eq!(doc.word_count(), 0);

        doc.content = "Hello world, this is a test.".to_string();
        assert_eq!(doc.word_count(), 6);
    }

    #[test]
    fn test_is_identity_document() {
        let identity = MemoryDocument::new("user1", None, paths::IDENTITY);
        assert!(identity.is_identity_document());

        let soul = MemoryDocument::new("user1", None, paths::SOUL);
        assert!(soul.is_identity_document());

        let memory = MemoryDocument::new("user1", None, paths::MEMORY);
        assert!(!memory.is_identity_document());

        let custom = MemoryDocument::new("user1", None, "projects/notes.md");
        assert!(!custom.is_identity_document());
    }

    #[test]
    fn test_workspace_entry_name() {
        let entry = WorkspaceEntry {
            path: "projects/alpha".to_string(),
            is_directory: true,
            updated_at: None,
            content_preview: None,
        };
        assert_eq!(entry.name(), "alpha");
    }

    #[test]
    fn test_merge_workspace_entries_empty() {
        let result = merge_workspace_entries(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_workspace_entries_keeps_newer_timestamp_and_preview() {
        use chrono::TimeZone;
        let old_ts = chrono::Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let new_ts = chrono::Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0).unwrap();

        let entries = vec![
            WorkspaceEntry {
                path: "notes.md".to_string(),
                is_directory: false,
                updated_at: Some(old_ts),
                content_preview: Some("old".to_string()),
            },
            WorkspaceEntry {
                path: "notes.md".to_string(),
                is_directory: false,
                updated_at: Some(new_ts),
                content_preview: Some("new".to_string()),
            },
        ];

        let result = merge_workspace_entries(entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].updated_at, Some(new_ts));
        assert_eq!(result[0].content_preview, Some("new".to_string()));
    }

    #[test]
    fn test_merge_workspace_entries_directory_wins() {
        let entries = vec![
            WorkspaceEntry {
                path: "projects".to_string(),
                is_directory: false,
                updated_at: None,
                content_preview: Some("file content".to_string()),
            },
            WorkspaceEntry {
                path: "projects".to_string(),
                is_directory: true,
                updated_at: None,
                content_preview: None,
            },
        ];

        let result = merge_workspace_entries(entries);
        assert_eq!(result.len(), 1);
        assert!(result[0].is_directory);
        assert!(result[0].content_preview.is_none());
    }

    #[test]
    fn test_merge_workspace_entries_fills_missing_timestamp() {
        use chrono::TimeZone;
        let ts = chrono::Utc.with_ymd_and_hms(2025, 3, 1, 0, 0, 0).unwrap();

        let entries = vec![
            WorkspaceEntry {
                path: "a.md".to_string(),
                is_directory: false,
                updated_at: None,
                content_preview: None,
            },
            WorkspaceEntry {
                path: "a.md".to_string(),
                is_directory: false,
                updated_at: Some(ts),
                content_preview: None,
            },
        ];

        let result = merge_workspace_entries(entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].updated_at, Some(ts));
    }

    #[test]
    fn test_document_metadata_default_is_empty() {
        let meta = DocumentMetadata::default();
        assert_eq!(meta.skip_indexing, None);
        assert_eq!(meta.skip_versioning, None);
        assert_eq!(meta.hygiene, None);
        assert!(meta.extra.is_empty());
    }

    #[test]
    fn test_document_metadata_from_value_full() {
        let value = serde_json::json!({
            "skip_indexing": true,
            "skip_versioning": false,
            "hygiene": { "enabled": true, "retention_days": 7 }
        });
        let meta = DocumentMetadata::from_value(&value);
        assert_eq!(meta.skip_indexing, Some(true));
        assert_eq!(meta.skip_versioning, Some(false));
        let hygiene = meta.hygiene.unwrap();
        assert!(hygiene.enabled);
        assert_eq!(hygiene.retention_days, 7);
    }

    #[test]
    fn test_document_metadata_from_value_partial() {
        let value = serde_json::json!({"skip_indexing": true});
        let meta = DocumentMetadata::from_value(&value);
        assert_eq!(meta.skip_indexing, Some(true));
        assert_eq!(meta.hygiene, None);
    }

    #[test]
    fn test_document_metadata_from_value_invalid() {
        let meta = DocumentMetadata::from_value(&serde_json::json!("not an object"));
        assert_eq!(meta, DocumentMetadata::default());
    }

    #[test]
    fn test_document_metadata_preserves_unknown_fields() {
        let value = serde_json::json!({
            "skip_indexing": true,
            "custom_field": "hello"
        });
        let meta = DocumentMetadata::from_value(&value);
        assert_eq!(meta.skip_indexing, Some(true));
        assert_eq!(
            meta.extra.get("custom_field").and_then(|v| v.as_str()),
            Some("hello")
        );

        // Round-trip preserves the field
        let back = meta.to_value();
        assert_eq!(
            back.get("custom_field").and_then(|v| v.as_str()),
            Some("hello")
        );
    }

    #[test]
    fn test_document_metadata_merge() {
        let base = serde_json::json!({"skip_indexing": false, "hygiene": {"enabled": true, "retention_days": 30}});
        let overlay = serde_json::json!({"skip_indexing": true, "skip_versioning": true});
        let merged = DocumentMetadata::merge(&base, &overlay);
        let meta = DocumentMetadata::from_value(&merged);
        // Overlay wins
        assert_eq!(meta.skip_indexing, Some(true));
        assert_eq!(meta.skip_versioning, Some(true));
        // Base preserved when not overridden
        assert!(meta.hygiene.is_some());
    }

    #[test]
    fn test_document_metadata_merge_empty_base() {
        let base = serde_json::json!({});
        let overlay = serde_json::json!({"skip_indexing": true});
        let merged = DocumentMetadata::merge(&base, &overlay);
        let meta = DocumentMetadata::from_value(&merged);
        assert_eq!(meta.skip_indexing, Some(true));
    }

    #[test]
    fn test_hygiene_metadata_default_retention() {
        let value = serde_json::json!({"enabled": true});
        let hygiene: HygieneMetadata = serde_json::from_value(value).unwrap();
        assert!(hygiene.enabled);
        assert_eq!(hygiene.retention_days, 30);
    }

    #[test]
    fn test_hygiene_metadata_retention_days_clamped_to_minimum() {
        // retention_days: 0 should be clamped to 1 to prevent mass-deletion.
        let hygiene: HygieneMetadata =
            serde_json::from_value(serde_json::json!({"enabled": true, "retention_days": 0}))
                .unwrap();
        assert_eq!(hygiene.retention_days, MIN_RETENTION_DAYS);

        // retention_days: 1 is the minimum and should pass through unchanged.
        let hygiene: HygieneMetadata =
            serde_json::from_value(serde_json::json!({"enabled": true, "retention_days": 1}))
                .unwrap();
        assert_eq!(hygiene.retention_days, 1);
    }

    #[test]
    fn test_content_sha256_deterministic() {
        let hash1 = content_sha256("hello world");
        let hash2 = content_sha256("hello world");
        assert_eq!(hash1, hash2);
        assert!(hash1.starts_with("sha256:"));
    }

    #[test]
    fn test_content_sha256_different_content() {
        let hash1 = content_sha256("hello");
        let hash2 = content_sha256("world");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_is_config_path() {
        assert!(is_config_path(".config"));
        assert!(is_config_path("daily/.config"));
        assert!(is_config_path("frontend/widgets/.config"));
        assert!(!is_config_path("daily/2024-01-15.md"));
        assert!(!is_config_path("MEMORY.md"));
        assert!(!is_config_path(".config.bak"));
    }

    #[test]
    fn test_is_config_path_edge_cases() {
        assert!(!is_config_path("foo.config"));
        assert!(!is_config_path(""));
        // ".config/bar" — the filename component is "bar", not ".config"
        assert!(!is_config_path(".config/bar"));
    }

    #[test]
    fn test_content_sha256_empty_string() {
        let hash = content_sha256("");
        assert!(hash.starts_with("sha256:"));
        // SHA-256 of "" is a known constant
        assert_eq!(
            hash,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_content_sha256_unicode() {
        let hash1 = content_sha256("Hello 🌍");
        let hash2 = content_sha256("Hello 🌍");
        assert_eq!(hash1, hash2);
        // Different unicode content produces different hashes
        let hash3 = content_sha256("Hello 🌎");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_document_metadata_merge_null_overlay_value() {
        // Overlay with null values should overwrite base values
        let base = serde_json::json!({"skip_indexing": true});
        let overlay = serde_json::json!({"skip_indexing": null});
        let merged = DocumentMetadata::merge(&base, &overlay);
        // null wins over true (shallow merge replaces entire key)
        assert_eq!(merged.get("skip_indexing"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn test_document_metadata_merge_nested_hygiene_replaced_wholesale() {
        // Nested objects are replaced entirely, not recursively merged
        let base = serde_json::json!({
            "hygiene": {"enabled": true, "retention_days": 30}
        });
        let overlay = serde_json::json!({
            "hygiene": {"enabled": false}
        });
        let merged = DocumentMetadata::merge(&base, &overlay);
        let meta = DocumentMetadata::from_value(&merged);
        let hygiene = meta.hygiene.unwrap();
        // Overlay replaced the entire hygiene object — retention_days falls back to default
        assert!(!hygiene.enabled);
        assert_eq!(hygiene.retention_days, 30); // serde default kicks in
    }

    #[test]
    fn test_document_metadata_merge_both_empty() {
        let merged = DocumentMetadata::merge(&serde_json::json!({}), &serde_json::json!({}));
        assert_eq!(merged, serde_json::json!({}));
    }

    #[test]
    fn test_document_metadata_merge_non_object_base() {
        // Non-object base is treated as empty
        let merged =
            DocumentMetadata::merge(&serde_json::json!("string"), &serde_json::json!({"a": 1}));
        assert_eq!(merged, serde_json::json!({"a": 1}));
    }

    #[test]
    fn test_merge_workspace_entries_sorted_by_path() {
        let entries = vec![
            WorkspaceEntry {
                path: "z.md".to_string(),
                is_directory: false,
                updated_at: None,
                content_preview: None,
            },
            WorkspaceEntry {
                path: "a.md".to_string(),
                is_directory: false,
                updated_at: None,
                content_preview: None,
            },
            WorkspaceEntry {
                path: "m.md".to_string(),
                is_directory: false,
                updated_at: None,
                content_preview: None,
            },
        ];

        let result = merge_workspace_entries(entries);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].path, "a.md");
        assert_eq!(result[1].path, "m.md");
        assert_eq!(result[2].path, "z.md");
    }
}
