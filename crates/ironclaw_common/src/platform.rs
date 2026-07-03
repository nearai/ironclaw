//! Runtime platform metadata injected into system prompts for self-awareness.
//!
//! Provides the agent with knowledge about its own identity and environment
//! so it can answer questions about itself, its capabilities, and its
//! configuration without relying on training data.

/// Runtime platform metadata.
#[derive(Debug, Clone, Default)]
pub struct PlatformInfo {
    /// Software version (from `CARGO_PKG_VERSION`).
    pub version: Option<String>,
    /// LLM backend name (e.g. "nearai", "openai", "anthropic").
    pub llm_backend: Option<String>,
    /// Active model name.
    pub model_name: Option<String>,
    /// Database backend (e.g. "libsql", "postgres").
    pub database_backend: Option<String>,
    /// Active channel names (e.g. ["telegram", "cli"]).
    pub active_channels: Vec<String>,
    /// Owner identifier.
    pub owner_id: Option<String>,
    /// Project repository URL.
    pub repo_url: Option<String>,
}

impl PlatformInfo {
    /// Format as a prompt section. Returns just the identity line if no other
    /// info is set.
    pub fn to_prompt_section(&self) -> String {
        let mut lines = Vec::new();

        lines.push("You are **IronClaw**, a secure autonomous AI assistant platform.".into());
        if let Some(ref v) = self.version {
            lines.push(format!("- Version: {v}"));
        }
        if let Some(ref repo) = self.repo_url {
            lines.push(format!("- Repository: {repo}"));
        }
        if let Some(ref owner) = self.owner_id {
            lines.push(format!("- Owner: {owner}"));
        }
        if let Some(ref backend) = self.llm_backend {
            let model = self.model_name.as_deref().unwrap_or("default");
            lines.push(format!("- LLM: {backend} ({model})"));
        }
        if let Some(ref db) = self.database_backend {
            lines.push(format!("- Database: {db}"));
        }
        if !self.active_channels.is_empty() {
            lines.push(format!("- Channels: {}", self.active_channels.join(", ")));
        }

        if lines.len() <= 1 {
            return format!("\n\n## Platform\n\n{}\n", lines[0]);
        }

        format!("\n\n## Platform\n\n{}\n", lines.join("\n"))
    }
}
