//! Read-only extraction layer for OpenClaw data.
//!
//! Handles opening OpenClaw SQLite databases and reading configuration
//! without making any modifications.

use std::fmt;
use std::path::{Path, PathBuf};

use secrecy::SecretString;

use crate::import::ImportError;

/// OpenClaw configuration structure (parsed from openclaw.json).
#[derive(Debug, Clone)]
pub struct OpenClawConfig {
    pub llm: Option<OpenClawLlmConfig>,
    pub embeddings: Option<OpenClawEmbeddingsConfig>,
    pub other_settings: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Clone)]
pub struct OpenClawLlmConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<SecretString>,
    pub base_url: Option<String>,
}

impl fmt::Debug for OpenClawLlmConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenClawLlmConfig")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "***REDACTED***"))
            .field("base_url", &self.base_url)
            .finish()
    }
}

#[derive(Clone)]
pub struct OpenClawEmbeddingsConfig {
    pub model: Option<String>,
    pub api_key: Option<SecretString>,
    pub provider: Option<String>,
}

impl fmt::Debug for OpenClawEmbeddingsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenClawEmbeddingsConfig")
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "***REDACTED***"))
            .field("provider", &self.provider)
            .finish()
    }
}

/// A memory chunk from OpenClaw's database.
#[derive(Debug, Clone)]
pub struct OpenClawMemoryChunk {
    pub path: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub chunk_index: i32,
}

/// A conversation from OpenClaw's database.
#[derive(Debug, Clone)]
pub struct OpenClawConversation {
    pub id: String,
    pub channel: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub messages: Vec<OpenClawMessage>,
}

/// A message within an OpenClaw conversation.
#[derive(Debug, Clone)]
pub struct OpenClawMessage {
    pub role: String,
    pub content: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Open an OpenClaw SQLite database file via libsql for read-only access.
#[cfg(feature = "import")]
async fn open_sqlite(db_path: &Path) -> Result<libsql::Connection, ImportError> {
    let db = libsql::Builder::new_local(db_path)
        .build()
        .await
        .map_err(|e| ImportError::Sqlite(e.to_string()))?;
    db.connect()
        .map_err(|e| ImportError::Sqlite(e.to_string()))
}

/// Reader for OpenClaw data files and databases.
pub struct OpenClawReader {
    openclaw_dir: PathBuf,
}

impl OpenClawReader {
    /// Create a new OpenClaw reader for the given directory.
    pub fn new(openclaw_dir: &Path) -> Result<Self, ImportError> {
        if !openclaw_dir.exists() {
            return Err(ImportError::NotFound {
                path: openclaw_dir.to_path_buf(),
                reason: "Directory does not exist".to_string(),
            });
        }

        Ok(Self {
            openclaw_dir: openclaw_dir.to_path_buf(),
        })
    }

    /// Check if an OpenClaw installation exists at ~/.openclaw.
    pub fn detect(home_dir: &Path) -> bool {
        let openclaw_dir = home_dir.join(".openclaw");
        let config_file = openclaw_dir.join("openclaw.json");
        config_file.exists()
    }

    /// Read and parse openclaw.json configuration.
    pub fn read_config(&self) -> Result<OpenClawConfig, ImportError> {
        let config_path = self.openclaw_dir.join("openclaw.json");

        if !config_path.exists() {
            return Err(ImportError::NotFound {
                path: config_path,
                reason: "openclaw.json not found".to_string(),
            });
        }

        let content = std::fs::read_to_string(&config_path).map_err(ImportError::Io)?;

        #[cfg(feature = "import")]
        {
            let config: serde_json::Value =
                json5::from_str(&content).map_err(|e| ImportError::ConfigParse(e.to_string()))?;

            // Extract LLM config
            let llm = config
                .get("llm")
                .and_then(|v| v.as_object())
                .map(|llm_obj| OpenClawLlmConfig {
                    provider: llm_obj
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    model: llm_obj
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    api_key: llm_obj
                        .get("api_key")
                        .and_then(|v| v.as_str())
                        .map(|s| SecretString::new(s.to_string().into_boxed_str())),
                    base_url: llm_obj
                        .get("base_url")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });

            // Extract embeddings config
            let embeddings = config
                .get("embeddings")
                .and_then(|v| v.as_object())
                .map(|emb_obj| OpenClawEmbeddingsConfig {
                    model: emb_obj
                        .get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    api_key: emb_obj
                        .get("api_key")
                        .and_then(|v| v.as_str())
                        .map(|s| SecretString::new(s.to_string().into_boxed_str())),
                    provider: emb_obj
                        .get("provider")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });

            // Store remaining settings
            let mut other_settings = std::collections::HashMap::new();
            if let Some(obj) = config.as_object() {
                for (k, v) in obj {
                    if k != "llm" && k != "embeddings" {
                        other_settings.insert(k.clone(), v.clone());
                    }
                }
            }

            Ok(OpenClawConfig {
                llm,
                embeddings,
                other_settings,
            })
        }

        #[cfg(not(feature = "import"))]
        {
            Err(ImportError::ConfigParse(
                "Import feature not enabled (compile with --features import)".to_string(),
            ))
        }
    }

    /// List all agent `.sqlite` files in the agents/ directory, sorted by name for deterministic order.
    pub fn list_agent_dbs(&self) -> Result<Vec<(String, PathBuf)>, ImportError> {
        let agents_dir = self.openclaw_dir.join("agents");

        if !agents_dir.exists() {
            // No agents directory is fine (might have no saved conversations)
            return Ok(Vec::new());
        }

        let mut dbs = Vec::new();
        for entry in std::fs::read_dir(&agents_dir).map_err(ImportError::Io)? {
            let entry = entry.map_err(ImportError::Io)?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("sqlite") {
                match path.file_stem().and_then(|s| s.to_str()) {
                    Some(name) => dbs.push((name.to_string(), path)),
                    None => {
                        tracing::warn!(
                            "Skipping agent database with non-UTF-8 filename: {:?}",
                            path
                        );
                    }
                }
            }
        }

        // Sort by agent name for deterministic ordering
        dbs.sort_by(|a, b| a.0.cmp(&b.0));

        Ok(dbs)
    }

    /// Read all memory chunks from an OpenClaw SQLite database.
    #[cfg(feature = "import")]
    pub async fn read_memory_chunks(
        &self,
        db_path: &Path,
    ) -> Result<Vec<OpenClawMemoryChunk>, ImportError> {
        let conn = open_sqlite(db_path).await?;

        let mut rows = conn
            .query("SELECT path, content, embedding, chunk_index FROM chunks", ())
            .await
            .map_err(|e| ImportError::Sqlite(e.to_string()))?;

        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| ImportError::Sqlite(e.to_string()))?
        {
            let path: String = row.get(0).map_err(|e| ImportError::Sqlite(e.to_string()))?;
            let content: String = row.get(1).map_err(|e| ImportError::Sqlite(e.to_string()))?;
            let embedding_blob: Option<Vec<u8>> =
                row.get(2).map_err(|e| ImportError::Sqlite(e.to_string()))?;
            let chunk_index: i32 = row.get(3).map_err(|e| ImportError::Sqlite(e.to_string()))?;

            // Convert binary embedding blob to Vec<f32> if present
            let embedding = embedding_blob.map(|bytes| {
                bytes
                    .chunks(4)
                    .map(|chunk| {
                        if chunk.len() == 4 {
                            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                        } else {
                            0.0
                        }
                    })
                    .collect()
            });

            result.push(OpenClawMemoryChunk {
                path,
                content,
                embedding,
                chunk_index,
            });
        }

        Ok(result)
    }

    /// Read all conversations from an OpenClaw SQLite database.
    #[cfg(feature = "import")]
    pub async fn read_conversations(
        &self,
        db_path: &Path,
    ) -> Result<Vec<OpenClawConversation>, ImportError> {
        let conn = open_sqlite(db_path).await?;

        let mut conv_rows = conn
            .query(
                "SELECT id, channel, created_at FROM conversations ORDER BY created_at DESC",
                (),
            )
            .await
            .map_err(|e| ImportError::Sqlite(e.to_string()))?;

        let mut conversations = Vec::new();
        while let Some(row) = conv_rows
            .next()
            .await
            .map_err(|e| ImportError::Sqlite(e.to_string()))?
        {
            let id: String = row.get(0).map_err(|e| ImportError::Sqlite(e.to_string()))?;
            let channel: String = row.get(1).map_err(|e| ImportError::Sqlite(e.to_string()))?;
            let created_at: Option<String> =
                row.get(2).map_err(|e| ImportError::Sqlite(e.to_string()))?;

            let created_at = created_at
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc));

            // Read messages for this conversation
            let mut msg_rows = conn
                .query(
                    "SELECT role, content, created_at FROM messages WHERE conversation_id = ?1 ORDER BY created_at",
                    libsql::params![id.as_str()],
                )
                .await
                .map_err(|e| ImportError::Sqlite(e.to_string()))?;

            let mut messages = Vec::new();
            while let Some(msg_row) = msg_rows
                .next()
                .await
                .map_err(|e| ImportError::Sqlite(e.to_string()))?
            {
                let role: String =
                    msg_row.get(0).map_err(|e| ImportError::Sqlite(e.to_string()))?;
                let content: String =
                    msg_row.get(1).map_err(|e| ImportError::Sqlite(e.to_string()))?;
                let msg_created_at: Option<String> =
                    msg_row.get(2).map_err(|e| ImportError::Sqlite(e.to_string()))?;

                let msg_created_at = msg_created_at
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                messages.push(OpenClawMessage {
                    role,
                    content,
                    created_at: msg_created_at,
                });
            }

            conversations.push(OpenClawConversation {
                id,
                channel,
                created_at,
                messages,
            });
        }

        Ok(conversations)
    }

    /// List workspace markdown files available for import.
    pub fn list_workspace_files(&self) -> Result<usize, ImportError> {
        let workspace_dir = self.openclaw_dir.join("workspace");

        if !workspace_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        if let Ok(entries) = std::fs::read_dir(&workspace_dir) {
            for entry in entries.flatten() {
                if let Some(ext) = entry.path().extension()
                    && ext == "md"
                {
                    count += 1;
                }
            }
        }

        Ok(count)
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn test_llm_config_debug_redacts_api_key() {
        let config = OpenClawLlmConfig {
            provider: Some("openai".to_string()),
            model: Some("gpt-4".to_string()),
            api_key: Some(SecretString::new("sk-secret-key-12345".into())),
            base_url: Some("https://api.openai.com".to_string()),
        };

        let debug_output = format!("{:?}", config);

        // Verify the actual API key is never exposed in debug output
        assert!(!debug_output.contains("sk-secret-key-12345"));
        // Verify the redaction marker is present
        assert!(debug_output.contains("***REDACTED***"));
    }

    #[test]
    fn test_embeddings_config_debug_redacts_api_key() {
        let config = OpenClawEmbeddingsConfig {
            model: Some("text-embedding-3-large".to_string()),
            api_key: Some(SecretString::new("sk-embed-secret-67890".into())),
            provider: Some("openai".to_string()),
        };

        let debug_output = format!("{:?}", config);

        // Verify the actual API key is never exposed in debug output
        assert!(!debug_output.contains("sk-embed-secret-67890"));
        // Verify the redaction marker is present
        assert!(debug_output.contains("***REDACTED***"));
    }

    #[test]
    fn test_llm_config_without_api_key() {
        let config = OpenClawLlmConfig {
            provider: Some("openai".to_string()),
            model: Some("gpt-4".to_string()),
            api_key: None,
            base_url: None,
        };

        let debug_output = format!("{:?}", config);

        // Should show None for missing API key
        assert!(debug_output.contains("api_key: None"));
    }
}
