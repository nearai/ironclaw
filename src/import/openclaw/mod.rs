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
    pub async fn import(&self) -> Result<ImportStats, ImportError> {
        let mut stats = ImportStats::default();

        // Create reader for OpenClaw data
        let reader = OpenClawReader::new(&self.opts.openclaw_path)?;

        // Step 1: Import settings and credentials
        let config = reader.read_config()?;
        let settings_map = settings::map_openclaw_config_to_settings(&config);

        if !self.opts.dry_run {
            for (key, value) in settings_map {
                if let Err(e) = self.db.set_setting(&self.opts.user_id, &key, &value).await {
                    tracing::warn!("Failed to import setting {}: {}", key, e);
                } else {
                    stats.settings += 1;
                }
            }
        } else {
            stats.settings = settings_map.len();
        }

        // Step 2: Import credentials
        let creds = settings::extract_credentials(&config);
        if !self.opts.dry_run {
            for (name, value) in creds {
                // SecretString cannot be used with CreateSecretParams, which expects a String
                // We need to extract the value from SecretString
                use secrecy::ExposeSecret;
                let exposed = value.expose_secret().to_string();
                let params = crate::secrets::CreateSecretParams::new(name, exposed);
                if let Err(e) = self.secrets.create(&self.opts.user_id, params).await {
                    tracing::warn!("Failed to import credential: {}", e);
                } else {
                    stats.secrets += 1;
                }
            }
        } else {
            stats.secrets = creds.len();
        }

        // Step 3: Import workspace documents (Markdown files)
        if let Ok(count) = reader.list_workspace_files() {
            if !self.opts.dry_run {
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
            } else {
                stats.documents = count;
            }
        }

        // Step 4: Import memory and history from agent databases
        let agent_dbs = reader.list_agent_dbs()?;
        for (_agent_name, db_path) in agent_dbs {
            // Import memory chunks
            match reader.read_memory_chunks(&db_path) {
                Ok(chunks) => {
                    for chunk in chunks {
                        if !self.opts.dry_run {
                            if let Err(e) = memory::import_chunk(&self.db, &chunk, &self.opts).await
                            {
                                tracing::warn!("Failed to import memory chunk: {}", e);
                            } else {
                                stats.chunks += 1;
                            }
                        } else {
                            stats.chunks += 1;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read memory chunks: {}", e);
                }
            }

            // Import conversations
            match reader.read_conversations(&db_path) {
                Ok(convs) => {
                    for conv in convs {
                        if !self.opts.dry_run {
                            match history::import_conversation(&self.db, conv, &self.opts).await {
                                Ok((_conv_id, msg_count)) => {
                                    stats.conversations += 1;
                                    stats.messages += msg_count;
                                }
                                Err(e) => {
                                    tracing::warn!("Failed to import conversation: {}", e);
                                }
                            }
                        } else {
                            stats.conversations += 1;
                            // Count messages in dry-run mode too
                            // For now, just count the conversation
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read conversations: {}", e);
                }
            }
        }

        Ok(stats)
    }
}
