//! Skill types, parsing, selection, and management for IronClaw.
//!
//! Skills are SKILL.md files (YAML frontmatter + markdown prompt) that extend the
//! agent's behavior through prompt-level instructions. This crate provides the core
//! types, SKILL.md parser, and filesystem management.
//!
//! # V2 Engine
//!
//! In the v2 engine, skill **selection and scoring** happen in the Python orchestrator
//! (`orchestrator/default.py`), not in Rust. The engine uses this crate only for:
//! - **`types`** + **`v2`** — Data structures (`SkillManifest`, `V2SkillMetadata`, etc.)
//! - **`parser`** — Parsing SKILL.md files during v1→v2 migration
//! - **`validation`** — Name/content escaping, credential spec validation
//!
//! # V1 Agent (remove after migration)
//!
//! The following modules are used **only by the v1 agent** (`src/agent/`). Once
//! the v1 agent is removed, they can be deleted or feature-gated:
//!
//! - **`selector`** — Rust-side deterministic scoring (`prefilter_skills`). In v2,
//!   the equivalent logic lives in `orchestrator/default.py:score_skill()`.
//! - **`gating`** — Binary/env/config requirement checks at load time. In v2,
//!   skills are stored as MemoryDocs and gating is not applicable.
//! - **`registry`** (feature-gated) — Filesystem discovery and install/remove.
//!   In v2, skills are managed as MemoryDocs via the Store.
//! - **`catalog`** (feature-gated) — ClawHub HTTP catalog. In v2, skill
//!   installation happens through the skill-extraction mission or direct API.
//!
//! # Trust Model
//!
//! Skills have two trust states that determine their authority:
//! - **Trusted**: User-placed skills (local/workspace) with full tool access
//! - **Installed**: Registry/external skills, restricted to read-only tools
//!
//! In v1, trust-based tool filtering happens via `src/skills/attenuation.rs`.
//! In v2, the Python orchestrator handles trust labels and the policy engine
//! controls tool access via capability leases.

pub mod gating;
pub mod install_metadata;
pub mod management;
mod parser;
mod selector;
pub mod types;
pub mod v2;
pub mod validation;

#[cfg(feature = "catalog")]
pub mod catalog;
#[cfg(feature = "registry")]
pub mod registry;

// Re-export core types at crate root for convenience.
pub use types::{
    ActivationCriteria, GatingRequirements, LoadedSkill, MAX_PROMPT_FILE_SIZE,
    ProviderRefreshStrategy, SkillCredentialLocation, SkillCredentialSpec, SkillManifest,
    SkillOAuthConfig, SkillSource, SkillTrust,
};

pub use gating::{GatingResult, check_requirements, check_requirements_sync};
pub use install_metadata::{
    INSTALL_METADATA_FILE_NAME, InstalledSkillMetadata, InstalledSkillMetadataSource,
    MAX_INSTALL_METADATA_BYTES,
};
pub use management::{
    MAX_INSTALL_BUNDLE_FILE_BYTES, MAX_INSTALL_BUNDLE_FILES, MAX_INSTALL_BUNDLE_TOTAL_BYTES,
    SkillContentRequest, SkillContentResult, SkillInstallFile, SkillInstallRequest,
    SkillInstallResult, SkillInstallSource, SkillManagementContext, SkillManagementError,
    SkillManagementErrorKind, SkillRemoveRequest, SkillRemoveResult, SkillSearchRequest,
    SkillSearchResult, SkillSource as ManagedSkillSource, SkillSummary, SkillUpdateRequest,
    SkillUpdateResult, install_skill, list_skills, read_skill_content, remove_skill, search_skills,
    skill_summary_json, update_skill,
};
pub use parser::{ParsedSkill, SkillParseError, parse_skill_md};
pub use selector::{
    MAX_SKILL_CONTEXT_TOKENS, SelectionOutcome, SkillSelectionOptions, extract_skill_mentions,
    prefilter_skills_with_options, skill_token_cost,
};
pub use validation::{
    SafeRelativePathError, escape_skill_content, escape_xml_attr, normalize_line_endings,
    normalize_safe_relative_path, validate_credential_name, validate_credential_spec,
    validate_path_pattern, validate_skill_name,
};

#[cfg(feature = "catalog")]
pub use catalog::{
    CatalogEntry, CatalogResolveError, CatalogSearchOutcome, SkillCatalog,
    catalog_entry_is_installed, resolve_catalog_slug_for_name, shared_catalog,
};
#[cfg(feature = "registry")]
pub use registry::{SkillRegistry, SkillRegistryError, compute_hash};
