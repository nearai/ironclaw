//! Skill system for shareable, prompt-level agent behaviors.
//!
//! Skills are TOML manifests containing instructions injected into the LLM context.
//! They can be loaded from GitHub repos, URLs, or local files and activated via
//! `/skill <name>` commands from any channel.
//!
//! # Security Architecture
//!
//! A skill IS text injected into the LLM's context, so a malicious skill IS a
//! prompt injection by design. Five defense layers protect against this:
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │ Layer 1: Static Analysis (load time)            │
//! │   Aho-Corasick patterns + skill-specific checks │
//! ├─────────────────────────────────────────────────┤
//! │ Layer 2: Hard Tool Whitelist (runtime)           │
//! │   Registry + execution level enforcement        │
//! ├─────────────────────────────────────────────────┤
//! │ Layer 3: Resource Restrictions (runtime)         │
//! │   Workspace paths, domains, tool call budget    │
//! ├─────────────────────────────────────────────────┤
//! │ Layer 4: User Approval Gate                      │
//! │   BLAKE3 hash pinning + full content review     │
//! ├─────────────────────────────────────────────────┤
//! │ Layer 5: Structural Prompt Isolation             │
//! │   <external_skill> wrapper + reassertion block  │
//! └─────────────────────────────────────────────────┘
//! ```

mod analyzer;
mod context;
mod loader;
mod manifest;
pub mod store;

pub use analyzer::{AnalysisVerdict, Finding, FindingCategory, SkillAnalyzer};
pub use context::{ActiveSkill, SkillContext};
pub use loader::SkillLoader;
pub use manifest::{ActivationMode, SkillManifest, SkillPermissions, SkillPrompt};
pub use store::{SkillApproval, SkillStore, StoredSkill};

/// Errors specific to the skill system.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    #[error("Skill '{name}' not found")]
    NotFound { name: String },

    #[error("Failed to parse skill manifest: {reason}")]
    ParseError { reason: String },

    #[error("Failed to load skill from {location}: {reason}")]
    LoadError { location: String, reason: String },

    #[error("Skill '{name}' blocked by static analysis: {reason}")]
    AnalysisBlocked { name: String, reason: String },

    #[error("Skill '{name}' requires re-approval (content changed)")]
    ApprovalInvalidated { name: String },

    #[error("Tool '{tool}' not allowed by skill '{skill}' whitelist")]
    ToolNotAllowed { tool: String, skill: String },

    #[error("Domain '{domain}' not allowed by skill '{skill}'")]
    DomainNotAllowed { domain: String, skill: String },

    #[error("Workspace path '{path}' not allowed by skill '{skill}'")]
    PathNotAllowed { path: String, skill: String },

    #[error("Tool call budget exhausted for skill '{skill}' (max {max})")]
    BudgetExhausted { skill: String, max: u32 },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Serialization error: {reason}")]
    Serialization { reason: String },
}
