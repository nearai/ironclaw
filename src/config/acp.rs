//! ACP (Agent Client Protocol) agent configuration.
//!
//! Stores configuration for ACP-compliant coding agents that can be spawned
//! as subprocesses inside Docker containers. Configuration is persisted at
//! `~/.ironclaw/acp-agents.json` (disk fallback) and in the database settings
//! table under key `"acp_agents"`.
//!
//! Mirrors the MCP server config pattern (`src/tools/mcp/config.rs`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::bootstrap::ironclaw_base_dir;

/// Configuration for a single ACP-compliant agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpAgentConfig {
    /// Unique name for this agent (e.g., "goose", "codex", "gemini").
    pub name: String,

    /// Command to spawn the agent subprocess.
    pub command: String,

    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,

    /// Additional environment variables for the agent process.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    /// Whether this agent is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Optional description for the agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn default_true() -> bool {
    true
}

impl AcpAgentConfig {
    /// Create a new ACP agent configuration.
    pub fn new(
        name: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args,
            env,
            enabled: true,
            description: None,
        }
    }

    /// Builder: attach a description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Validate the agent configuration.
    pub fn validate(&self) -> Result<(), AcpConfigError> {
        if self.name.is_empty() {
            return Err(AcpConfigError::InvalidConfig {
                reason: "agent name cannot be empty".to_string(),
            });
        }
        if self.command.is_empty() {
            return Err(AcpConfigError::InvalidConfig {
                reason: format!("agent '{}': command cannot be empty", self.name),
            });
        }
        Ok(())
    }
}

/// Container for all configured ACP agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpAgentsFile {
    /// List of configured ACP agents.
    #[serde(default)]
    pub agents: Vec<AcpAgentConfig>,

    /// Schema version for future compatibility.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
}

fn default_schema_version() -> u32 {
    1
}

impl Default for AcpAgentsFile {
    fn default() -> Self {
        Self {
            agents: Vec::new(),
            schema_version: default_schema_version(),
        }
    }
}

impl AcpAgentsFile {
    /// Get an agent by name.
    pub fn get(&self, name: &str) -> Option<&AcpAgentConfig> {
        self.agents.iter().find(|a| a.name == name)
    }

    /// Get a mutable agent by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut AcpAgentConfig> {
        self.agents.iter_mut().find(|a| a.name == name)
    }

    /// Add or update an agent configuration.
    pub fn upsert(&mut self, config: AcpAgentConfig) {
        if let Some(existing) = self.get_mut(&config.name) {
            *existing = config;
        } else {
            self.agents.push(config);
        }
    }

    /// Remove an agent by name.
    pub fn remove(&mut self, name: &str) -> bool {
        let len_before = self.agents.len();
        self.agents.retain(|a| a.name != name);
        self.agents.len() < len_before
    }

    /// Get all enabled agents.
    pub fn enabled_agents(&self) -> impl Iterator<Item = &AcpAgentConfig> {
        self.agents.iter().filter(|a| a.enabled)
    }
}

/// Error type for ACP configuration operations.
#[derive(Debug, thiserror::Error)]
pub enum AcpConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Invalid configuration: {reason}")]
    InvalidConfig { reason: String },

    #[error("Agent not found: {name}")]
    AgentNotFound { name: String },

    #[error("Agent is disabled: {name}")]
    AgentDisabled { name: String },
}

// ==================== Disk persistence ====================

/// Get the default ACP agents configuration path.
pub fn default_config_path() -> PathBuf {
    ironclaw_base_dir().join("acp-agents.json")
}

/// Load ACP agent configurations from the default location.
pub async fn load_acp_agents() -> Result<AcpAgentsFile, AcpConfigError> {
    load_acp_agents_from(default_config_path()).await
}

/// Load ACP agent configurations from a specific path.
pub async fn load_acp_agents_from(path: impl AsRef<Path>) -> Result<AcpAgentsFile, AcpConfigError> {
    let path = path.as_ref();

    let content = match fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AcpAgentsFile::default());
        }
        Err(e) => return Err(e.into()),
    };
    let config: AcpAgentsFile = serde_json::from_str(&content)?;

    for agent in &config.agents {
        agent
            .validate()
            .map_err(|e| AcpConfigError::InvalidConfig {
                reason: format!("Agent '{}': {}", agent.name, e),
            })?;
    }

    Ok(config)
}

/// Save ACP agent configurations to the default location.
pub async fn save_acp_agents(config: &AcpAgentsFile) -> Result<(), AcpConfigError> {
    save_acp_agents_to(config, default_config_path()).await
}

/// Save ACP agent configurations to a specific path.
pub async fn save_acp_agents_to(
    config: &AcpAgentsFile,
    path: impl AsRef<Path>,
) -> Result<(), AcpConfigError> {
    let path = path.as_ref();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let content = serde_json::to_string_pretty(config)?;

    // Atomic write via temp file to avoid corruption on crash.
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content).await?;
    fs::rename(&tmp_path, path).await?;

    Ok(())
}

/// Add a new ACP agent configuration (disk-backed).
pub async fn add_acp_agent(config: AcpAgentConfig) -> Result<(), AcpConfigError> {
    config.validate()?;

    let mut agents = load_acp_agents().await?;
    agents.upsert(config);
    save_acp_agents(&agents).await?;

    Ok(())
}

/// Remove an ACP agent by name (disk-backed).
pub async fn remove_acp_agent(name: &str) -> Result<(), AcpConfigError> {
    let mut agents = load_acp_agents().await?;

    if !agents.remove(name) {
        return Err(AcpConfigError::AgentNotFound {
            name: name.to_string(),
        });
    }

    save_acp_agents(&agents).await?;

    Ok(())
}

/// Get a specific ACP agent configuration (disk-backed).
pub async fn get_acp_agent(name: &str) -> Result<AcpAgentConfig, AcpConfigError> {
    let agents = load_acp_agents().await?;

    agents
        .get(name)
        .cloned()
        .ok_or_else(|| AcpConfigError::AgentNotFound {
            name: name.to_string(),
        })
}

// ==================== Database-backed persistence ====================

/// Load ACP agent configurations from the database settings table.
///
/// Falls back to the disk file only if DB has no entry.
pub async fn load_acp_agents_from_db(
    store: &dyn crate::db::Database,
    user_id: &str,
) -> Result<AcpAgentsFile, AcpConfigError> {
    match store.get_setting(user_id, "acp_agents").await {
        Ok(Some(value)) => {
            let config: AcpAgentsFile = serde_json::from_value(value)?;
            for agent in &config.agents {
                agent
                    .validate()
                    .map_err(|e| AcpConfigError::InvalidConfig {
                        reason: format!("Agent '{}': {}", agent.name, e),
                    })?;
            }
            Ok(config)
        }
        Ok(None) => load_acp_agents().await,
        Err(e) => Err(AcpConfigError::Database(e.to_string())),
    }
}

/// Load ACP agent configurations from the active persistence backend.
pub async fn load_acp_agents_for_user(
    store: Option<&dyn crate::db::Database>,
    user_id: &str,
) -> Result<AcpAgentsFile, AcpConfigError> {
    match store {
        Some(store) => load_acp_agents_from_db(store, user_id).await,
        None => load_acp_agents().await,
    }
}

/// Save ACP agent configurations to the database settings table.
pub async fn save_acp_agents_to_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    config: &AcpAgentsFile,
) -> Result<(), AcpConfigError> {
    let value = serde_json::to_value(config)?;
    store
        .set_setting(user_id, "acp_agents", &value)
        .await
        .map_err(std::io::Error::other)?;
    Ok(())
}

/// Save ACP agent configurations to the active persistence backend.
pub async fn save_acp_agents_for_user(
    store: Option<&dyn crate::db::Database>,
    user_id: &str,
    config: &AcpAgentsFile,
) -> Result<(), AcpConfigError> {
    match store {
        Some(store) => save_acp_agents_to_db(store, user_id, config).await,
        None => save_acp_agents(config).await,
    }
}

/// Add a new ACP agent configuration (DB-backed).
pub async fn add_acp_agent_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    config: AcpAgentConfig,
) -> Result<(), AcpConfigError> {
    config.validate()?;

    let mut agents = load_acp_agents_from_db(store, user_id).await?;
    agents.upsert(config);
    save_acp_agents_to_db(store, user_id, &agents).await?;

    Ok(())
}

/// Remove an ACP agent by name (DB-backed).
pub async fn remove_acp_agent_db(
    store: &dyn crate::db::Database,
    user_id: &str,
    name: &str,
) -> Result<(), AcpConfigError> {
    let mut agents = load_acp_agents_from_db(store, user_id).await?;

    if !agents.remove(name) {
        return Err(AcpConfigError::AgentNotFound {
            name: name.to_string(),
        });
    }

    save_acp_agents_to_db(store, user_id, &agents).await?;
    Ok(())
}

/// Load a single ACP agent from the active persistence backend.
pub async fn get_acp_agent_for_user(
    store: Option<&dyn crate::db::Database>,
    user_id: &str,
    name: &str,
) -> Result<AcpAgentConfig, AcpConfigError> {
    let agents = load_acp_agents_for_user(store, user_id).await?;
    agents
        .get(name)
        .cloned()
        .ok_or_else(|| AcpConfigError::AgentNotFound {
            name: name.to_string(),
        })
}

/// Load a single ACP agent and ensure it is enabled.
pub async fn get_enabled_acp_agent_for_user(
    store: Option<&dyn crate::db::Database>,
    user_id: &str,
    name: &str,
) -> Result<AcpAgentConfig, AcpConfigError> {
    let agent = get_acp_agent_for_user(store, user_id, name).await?;
    if !agent.enabled {
        return Err(AcpConfigError::AgentDisabled {
            name: name.to_string(),
        });
    }
    Ok(agent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_config_new() {
        let agent = AcpAgentConfig::new("goose", "goose", vec!["--stdio".into()], HashMap::new());
        assert_eq!(agent.name, "goose");
        assert_eq!(agent.command, "goose");
        assert_eq!(agent.args, vec!["--stdio"]);
        assert!(agent.enabled);
        assert!(agent.description.is_none());
    }

    #[test]
    fn test_agent_config_with_description() {
        let agent = AcpAgentConfig::new("goose", "goose", vec![], HashMap::new())
            .with_description("Goose coding agent");
        assert_eq!(agent.description.as_deref(), Some("Goose coding agent"));
    }

    #[test]
    fn test_validate_empty_name() {
        let agent = AcpAgentConfig::new("", "goose", vec![], HashMap::new());
        assert!(agent.validate().is_err());
    }

    #[test]
    fn test_validate_empty_command() {
        let agent = AcpAgentConfig::new("goose", "", vec![], HashMap::new());
        assert!(agent.validate().is_err());
    }

    #[test]
    fn test_validate_ok() {
        let agent = AcpAgentConfig::new("goose", "goose", vec!["--stdio".into()], HashMap::new());
        assert!(agent.validate().is_ok());
    }

    #[test]
    fn test_agents_file_default() {
        let file = AcpAgentsFile::default();
        assert!(file.agents.is_empty());
        assert_eq!(file.schema_version, 1);
    }

    #[test]
    fn test_agents_file_get() {
        let mut file = AcpAgentsFile::default();
        file.agents.push(AcpAgentConfig::new(
            "goose",
            "goose",
            vec![],
            HashMap::new(),
        ));
        assert!(file.get("goose").is_some());
        assert!(file.get("nonexistent").is_none());
    }

    #[test]
    fn test_agents_file_upsert_new() {
        let mut file = AcpAgentsFile::default();
        file.upsert(AcpAgentConfig::new(
            "goose",
            "goose",
            vec![],
            HashMap::new(),
        ));
        assert_eq!(file.agents.len(), 1);
    }

    #[test]
    fn test_agents_file_upsert_existing() {
        let mut file = AcpAgentsFile::default();
        file.upsert(AcpAgentConfig::new(
            "goose",
            "goose",
            vec![],
            HashMap::new(),
        ));
        file.upsert(AcpAgentConfig::new(
            "goose",
            "goose-v2",
            vec!["--stdio".into()],
            HashMap::new(),
        ));
        assert_eq!(file.agents.len(), 1);
        assert_eq!(file.agents[0].command, "goose-v2");
    }

    #[test]
    fn test_agents_file_remove() {
        let mut file = AcpAgentsFile::default();
        file.upsert(AcpAgentConfig::new(
            "goose",
            "goose",
            vec![],
            HashMap::new(),
        ));
        assert!(file.remove("goose"));
        assert!(file.agents.is_empty());
        assert!(!file.remove("goose")); // already removed
    }

    #[test]
    fn test_agents_file_enabled_agents() {
        let mut file = AcpAgentsFile::default();
        file.upsert(AcpAgentConfig::new(
            "goose",
            "goose",
            vec![],
            HashMap::new(),
        ));
        let mut disabled = AcpAgentConfig::new("codex", "codex", vec![], HashMap::new());
        disabled.enabled = false;
        file.upsert(disabled);

        let enabled: Vec<_> = file.enabled_agents().collect();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "goose");
    }

    #[test]
    fn test_agents_file_serde_roundtrip() {
        let mut file = AcpAgentsFile::default();
        file.upsert(AcpAgentConfig::new(
            "goose",
            "goose",
            vec!["--stdio".into()],
            HashMap::from([("GOOSE_TOKEN".to_string(), "secret".to_string())]),
        ));

        let json = serde_json::to_string_pretty(&file).unwrap();
        let parsed: AcpAgentsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.agents.len(), 1);
        assert_eq!(parsed.agents[0].name, "goose");
        assert_eq!(parsed.agents[0].command, "goose");
        assert_eq!(parsed.agents[0].args, vec!["--stdio"]);
        assert!(parsed.agents[0].env.contains_key("GOOSE_TOKEN"));
    }

    #[test]
    fn test_default_config_path() {
        let path = default_config_path();
        assert!(path.ends_with("acp-agents.json"));
    }

    #[tokio::test]
    async fn test_load_nonexistent_returns_empty() {
        let path = std::env::temp_dir().join("ironclaw-test-nonexistent-acp.json");
        let _ = std::fs::remove_file(&path); // ensure clean
        let file = load_acp_agents_from(&path).await.unwrap();
        assert!(file.agents.is_empty());
    }

    #[tokio::test]
    async fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("acp-agents.json");

        let mut file = AcpAgentsFile::default();
        file.upsert(AcpAgentConfig::new(
            "goose",
            "goose",
            vec!["--stdio".into()],
            HashMap::new(),
        ));

        save_acp_agents_to(&file, &path).await.unwrap();
        let loaded = load_acp_agents_from(&path).await.unwrap();
        assert_eq!(loaded.agents.len(), 1);
        assert_eq!(loaded.agents[0].name, "goose");
    }

    #[tokio::test]
    async fn test_load_rejects_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("acp-agents.json");
        tokio::fs::write(
            &path,
            r#"{"agents":[{"name":"","command":"goose","args":[]}]}"#,
        )
        .await
        .unwrap();

        let result = load_acp_agents_from(&path).await;
        assert!(result.is_err());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_load_and_save_acp_agents_for_non_default_owner_scope() {
        let (db, _tmp) = crate::testing::test_db().await;

        let mut file = AcpAgentsFile::default();
        file.upsert(AcpAgentConfig::new(
            "goose",
            "goose",
            vec!["--stdio".into()],
            HashMap::new(),
        ));

        save_acp_agents_for_user(Some(db.as_ref()), "owner-123", &file)
            .await
            .unwrap();

        let loaded = load_acp_agents_for_user(Some(db.as_ref()), "owner-123")
            .await
            .unwrap();
        assert_eq!(
            loaded.get("goose").map(|agent| agent.command.as_str()),
            Some("goose")
        );

        let default_scope = load_acp_agents_for_user(Some(db.as_ref()), "default")
            .await
            .unwrap();
        assert!(default_scope.get("goose").is_none());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_get_enabled_agent_rejects_disabled_agent() {
        let (db, _tmp) = crate::testing::test_db().await;

        let mut file = AcpAgentsFile::default();
        let mut agent = AcpAgentConfig::new("codex", "codex", vec!["acp".into()], HashMap::new());
        agent.enabled = false;
        file.upsert(agent);

        save_acp_agents_for_user(Some(db.as_ref()), "owner-123", &file)
            .await
            .unwrap();

        let err = get_enabled_acp_agent_for_user(Some(db.as_ref()), "owner-123", "codex")
            .await
            .unwrap_err();
        assert!(matches!(err, AcpConfigError::AgentDisabled { .. }));
    }
}
