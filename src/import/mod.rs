//! Import data from other personal AI assistants.
//!
//! Currently supports importing from OpenClaw installations, including:
//! - Memory documents (identity files, workspace memory)
//! - Conversation history (JSONL session transcripts)
//! - Settings (config key mapping)
//! - Credentials (API keys, OAuth tokens)

mod config_parser;
mod discovery;
mod openclaw;
mod progress;

pub use config_parser::{CredentialEntry, OpenClawConfig};
pub use discovery::OpenClawInstallation;
pub use openclaw::OpenClawImporter;
pub use progress::{ImportProgress, SilentProgress, TerminalProgress};

/// Errors that can occur during data import.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("No installation found at the expected path")]
    NotFound,

    #[error("Failed to parse configuration: {0}")]
    ConfigParse(String),

    #[error("Failed to parse session file: {0}")]
    SessionParse(String),

    #[error("Database write failed: {0}")]
    DatabaseWrite(String),

    #[error("Secrets error: {0}")]
    Secrets(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Summary of an import operation.
#[derive(Debug, Default)]
pub struct ImportReport {
    /// Number of settings imported.
    pub settings_count: usize,
    /// Number of memory documents imported.
    pub memory_documents: usize,
    /// Number of identity files imported.
    pub identity_files: usize,
    /// Number of conversations imported.
    pub conversations: usize,
    /// Number of messages imported.
    pub messages: usize,
    /// Number of credentials imported.
    pub credentials: usize,
    /// Number of items skipped because they already exist.
    pub skipped_already_exists: usize,
    /// Per-item errors that did not abort the import.
    pub errors: Vec<String>,
    /// Whether this was a dry run (no writes).
    pub dry_run: bool,
}

impl ImportReport {
    /// Print a human-readable summary to stdout.
    pub fn print_summary(&self) {
        println!();
        if self.dry_run {
            println!("=== Import Dry Run Summary ===");
            println!("(No data was written)");
        } else {
            println!("=== Import Summary ===");
        }
        println!();

        let verb = if self.dry_run {
            "would import"
        } else {
            "imported"
        };

        if self.settings_count > 0 {
            println!("  Settings:       {} {}", self.settings_count, verb);
        }
        if self.identity_files > 0 {
            println!("  Identity files: {} {}", self.identity_files, verb);
        }
        if self.memory_documents > 0 {
            println!("  Memory docs:    {} {}", self.memory_documents, verb);
        }
        if self.conversations > 0 {
            println!(
                "  Conversations:  {} {} ({} messages)",
                self.conversations, verb, self.messages
            );
        }
        if self.credentials > 0 {
            println!("  Credentials:    {} {}", self.credentials, verb);
        }
        if self.skipped_already_exists > 0 {
            println!(
                "  Skipped:        {} (already exist)",
                self.skipped_already_exists
            );
        }

        let total = self.settings_count
            + self.identity_files
            + self.memory_documents
            + self.conversations
            + self.credentials;
        if total == 0 && self.skipped_already_exists == 0 {
            println!("  Nothing to import.");
        }

        if !self.errors.is_empty() {
            println!();
            println!("  Errors ({}):", self.errors.len());
            for (i, err) in self.errors.iter().enumerate() {
                println!("    {}. {}", i + 1, err);
            }
        }

        println!();
    }
}
