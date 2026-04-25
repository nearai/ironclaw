//! OpenClaw migration and import functionality.
//!
//! Provides tools to migrate existing OpenClaw installations (memory, history,
//! settings, and credentials) into IronClaw without data loss.

#[cfg(feature = "migrate")]
pub mod openclaw;

use std::path::PathBuf;

/// Configuration options for OpenClaw import.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Path to the OpenClaw directory (default: ~/.openclaw).
    pub openclaw_path: PathBuf,
    /// Dry-run mode: report what would be imported without writing to DB.
    pub dry_run: bool,
    /// Re-embed memory documents if dimension mismatch detected.
    pub re_embed: bool,
    /// User ID for scoping imported data.
    pub user_id: String,
}

/// Statistics collected during an import operation.
#[derive(Debug, Clone, Default)]
pub struct ImportStats {
    /// Number of workspace documents imported.
    pub documents: usize,
    /// Number of memory chunks imported.
    pub chunks: usize,
    /// Number of conversations imported.
    pub conversations: usize,
    /// Number of messages imported.
    pub messages: usize,
    /// Number of settings imported.
    pub settings: usize,
    /// Number of credentials imported.
    pub secrets: usize,
    /// Number of items skipped (already existed).
    pub skipped: usize,
    /// Number of chunks queued for re-embedding.
    pub re_embed_queued: usize,
}

impl ImportStats {
    /// Check if any items were imported.
    pub fn is_empty(&self) -> bool {
        self.documents == 0
            && self.chunks == 0
            && self.conversations == 0
            && self.messages == 0
            && self.settings == 0
            && self.secrets == 0
    }

    /// Total number of items imported.
    pub fn total_imported(&self) -> usize {
        self.documents
            + self.chunks
            + self.conversations
            + self.messages
            + self.settings
            + self.secrets
    }
}

/// Errors that can occur during import.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("OpenClaw not found at {path}: {reason}")]
    NotFound { path: PathBuf, reason: String },

    #[error("JSON5 parse error: {0}")]
    ConfigParse(String),

    #[error("SQLite error: {0}")]
    Sqlite(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Workspace error: {0}")]
    Workspace(String),

    #[error("Secret error: {0}")]
    Secret(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid UTF-8: {0}")]
    InvalidUtf8(String),
}
