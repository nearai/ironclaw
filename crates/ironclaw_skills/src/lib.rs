//! Skill selection, scoring, and management for IronClaw.
//!
//! Skills are SKILL.md files (YAML frontmatter + markdown prompt) that extend the
//! agent's behavior through prompt-level instructions. This crate provides the core
//! types, deterministic selection pipeline, and filesystem management.
//!
//! # Trust Model
//!
//! Skills have two trust states that determine their authority:
//! - **Trusted**: User-placed skills (local/workspace) with full tool access
//! - **Installed**: Registry/external skills, restricted to read-only tools
//!
//! The effective tool ceiling is determined by the *lowest-trust* active skill,
//! preventing privilege escalation through skill mixing.

pub mod gating;
pub mod parser;
pub mod selector;
pub mod types;
pub mod v2;
pub mod validation;

#[cfg(feature = "catalog")]
pub mod catalog;
#[cfg(feature = "registry")]
pub mod registry;

// Re-export core types at crate root for convenience.
pub use types::{
    ActivationCriteria, GatingRequirements, LoadedSkill, OpenClawMeta, SkillManifest,
    SkillMetadata, SkillSource, SkillTrust, MAX_PROMPT_FILE_SIZE,
};

pub use parser::{ParsedSkill, SkillParseError, parse_skill_md};
pub use selector::{prefilter_skills, MAX_SKILL_CONTEXT_TOKENS};
pub use validation::{escape_skill_content, escape_xml_attr, normalize_line_endings, validate_skill_name};
pub use gating::{GatingResult, check_requirements, check_requirements_sync};

#[cfg(feature = "registry")]
pub use registry::{SkillRegistry, SkillRegistryError, compute_hash};
#[cfg(feature = "catalog")]
pub use catalog::{CatalogEntry, CatalogSearchOutcome, SkillCatalog, shared_catalog};
