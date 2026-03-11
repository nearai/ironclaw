//! OpenClaw data migration orchestration and detection.

pub mod credentials;
pub mod history;
pub mod memory;
pub mod reader;
pub mod settings;

use std::path::PathBuf;
use std::sync::Arc;

use crate::db::Database;
use crate::import::{ImportError, ImportOptions, ImportStats};
use crate::secrets::SecretsStore;
use crate::workspace::Workspace;

pub use reader::OpenClawReader;

/// OpenClaw importer that coordinates migration of all data types.
pub struct OpenClawImporter {
    db: Arc<dyn Database>,
    workspace: Workspace,
    secrets: Arc<dyn SecretsStore>,
    opts: ImportOptions,
}

impl OpenClawImporter {
    /// Create a new OpenClaw importer.
    pub fn new(
        db: Arc<dyn Database>,
        workspace: Workspace,
        secrets: Arc<dyn SecretsStore>,
        opts: ImportOptions,
    ) -> Self {
        Self {
            db,
            workspace,
            secrets,
            opts,
        }
    }

    /// Detect if an OpenClaw installation exists at the default location (~/.openclaw).
    pub fn detect() -> Option<PathBuf> {
        if let Ok(home) = std::env::var("HOME") {
            let openclaw_dir = PathBuf::from(home).join(".openclaw");
            let config_file = openclaw_dir.join("openclaw.json");
            if config_file.exists() {
                return Some(openclaw_dir);
            }
        }
        None
    }

    /// Run the import process for all data types.
    ///
    /// Returns detailed statistics about what was imported.
    /// If `dry_run` is enabled, no data is written to the database.
    ///
    /// **Database Safety Note:** The Database trait does not currently expose explicit
    /// transaction control (BEGIN/COMMIT/ROLLBACK). To minimize consistency risks:
    /// - All configuration reading is done before any writes
    /// - Writes are grouped by type (settings, credentials, documents, chunks, conversations)
    /// - Conversations are handled atomically: creation + all messages added together
    /// - Errors are logged but don't stop the entire import (fail-safe behavior)
    pub async fn import(&self) -> Result<ImportStats, ImportError> {
        let mut stats = ImportStats::default();

        // === PHASE 1: READ ALL DATA BEFORE ANY WRITES ===
        // This minimizes the window where the database could be left in a partial state

        // Read OpenClaw data
        let reader = OpenClawReader::new(&self.opts.openclaw_path)?;
        let config = reader.read_config()?;
        let agent_dbs = reader.list_agent_dbs()?;

        // Pre-read all conversation data to validate before writing
        let mut all_conversations = Vec::new();
        for (_agent_name, db_path) in &agent_dbs {
            match reader.read_conversations(db_path) {
                Ok(convs) => all_conversations.extend(convs),
                Err(e) => {
                    tracing::warn!("Failed to read conversations: {}", e);
                }
            }
        }

        // Pre-read all memory chunks
        let mut all_chunks = Vec::new();
        for (_agent_name, db_path) in &agent_dbs {
            match reader.read_memory_chunks(db_path) {
                Ok(chunks) => all_chunks.extend(chunks),
                Err(e) => {
                    tracing::warn!("Failed to read memory chunks: {}", e);
                }
            }
        }

        // Prepare all settings and credentials
        let settings_map = settings::map_openclaw_config_to_settings(&config);
        let creds = settings::extract_credentials(&config);

        // === PHASE 2: WRITE IN GROUPED ORDER ===
        // If a crash occurs, earlier groups are fully committed

        if !self.opts.dry_run {
            // Group 1: Settings (should be idempotent via upsert)
            for (key, value) in settings_map {
                if let Err(e) = self.db.set_setting(&self.opts.user_id, &key, &value).await {
                    tracing::warn!("Failed to import setting {}: {}", key, e);
                } else {
                    stats.settings += 1;
                }
            }

            // Group 2: Credentials (should be idempotent via upsert)
            for (name, value) in creds {
                use secrecy::ExposeSecret;
                let exposed = value.expose_secret().to_string();
                let params = crate::secrets::CreateSecretParams::new(name, exposed);
                if let Err(e) = self.secrets.create(&self.opts.user_id, params).await {
                    tracing::warn!("Failed to import credential: {}", e);
                } else {
                    stats.secrets += 1;
                }
            }

            // Group 3: Workspace documents
            if let Ok(_count) = reader.list_workspace_files() {
                match self
                    .workspace
                    .import_from_directory(&self.opts.openclaw_path.join("workspace"))
                    .await
                {
                    Ok(imported) => stats.documents = imported,
                    Err(e) => {
                        tracing::warn!("Failed to import workspace documents: {}", e);
                    }
                }
            }

            // Group 4: Memory chunks (should be idempotent via path deduplication)
            for chunk in all_chunks {
                if let Err(e) = memory::import_chunk(&self.db, &chunk, &self.opts).await {
                    tracing::warn!("Failed to import memory chunk: {}", e);
                } else {
                    stats.chunks += 1;
                }
            }

            // Group 5: Conversations with messages
            // CRITICAL: Each conversation + its messages form an atomic unit.
            // If a crash occurs mid-conversation, only that conversation is incomplete.
            // All previous conversations are fully committed.
            for conv in all_conversations {
                match history::import_conversation_atomic(&self.db, conv, &self.opts).await {
                    Ok((_conv_id, msg_count)) => {
                        stats.conversations += 1;
                        stats.messages += msg_count;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to import conversation: {}", e);
                    }
                }
            }
        } else {
            // DRY RUN: Count only
            stats.settings = settings_map.len();
            stats.secrets = creds.len();
            if let Ok(count) = reader.list_workspace_files() {
                stats.documents = count;
            }
            stats.chunks = all_chunks.len();
            stats.conversations = all_conversations.len();
            for conv in &all_conversations {
                stats.messages += conv.messages.len();
            }
        }

        Ok(stats)
    }
}
