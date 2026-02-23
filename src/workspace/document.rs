//! Memory document types for the workspace.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
        matches!(
            self.path.as_str(),
            paths::IDENTITY | paths::SOUL | paths::AGENTS | paths::USER
        )
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
                if let (Some(existing_ts), Some(new_ts)) =
                    (&existing.updated_at, &entry.updated_at)
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
