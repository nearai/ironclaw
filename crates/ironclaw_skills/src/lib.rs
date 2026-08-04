//! Skill types, parsing, selection, and management for IronClaw.
//!
//! Skills are SKILL.md files (YAML frontmatter + markdown prompt) that extend the
//! agent's behavior through prompt-level instructions. This crate provides the core
//! types, SKILL.md parser, and filesystem management.
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

/// Hot-swappable skill-activation strategies (profile `skill.activation.v1`).
///
/// Mirrors the memory-provider binding pattern: named strategies, fail-closed
/// resolution, behavior-preserving default. See the module docs for why an
/// agent-authored skill is unreachable under the historical criteria-only rule.
pub mod activation_strategy;
pub mod gating;
pub use gating::{GatingResult, binary_exists, check_requirements_sync};
pub mod install_metadata;
pub mod learning;
pub mod management;
mod parser;
pub mod scoped_management;
mod selector;
pub mod types;
pub mod validation;

// Re-export core types at crate root for convenience.
pub use types::{
    ActivationCriteria, GatingRequirements, LoadedSkill, MAX_PROMPT_FILE_SIZE,
    ProviderRefreshStrategy, SkillCredentialLocation, SkillCredentialSpec, SkillManifest,
    SkillOAuthConfig, SkillSource, SkillTrust,
};

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
pub use parser::{ParsedSkill, SkillParseError, parse_skill_md, set_skill_auto_activate};
pub use scoped_management::{
    ScopedSkillManagementBuildError, ScopedSkillManagementError,
    ScopedSkillManagementMountResolver, ScopedSkillManagementPort, SkillReplacementSnapshot,
    build_existing_standalone_skill_management_port, build_scoped_skill_management_port,
};
pub use selector::{
    MAX_SKILL_CONTEXT_TOKENS, SelectionOutcome, SkillSelectionOptions, extract_skill_mentions,
    prefilter_skills_with_options, skill_token_cost,
};
pub use validation::{
    lint_skill_routing_metadata, lint_skill_routing_metadata_advisory,
    lint_skill_routing_metadata_blocking,
    SafeRelativePathError, escape_skill_content, escape_xml_attr, normalize_line_endings,
    normalize_safe_relative_path, validate_credential_name, validate_credential_spec,
    validate_path_pattern, validate_skill_name,
};
#[cfg(test)]
mod replacement_snapshot_public_surface_tests {
    #[test]
    fn replacement_snapshot_is_exported_at_the_crate_root() {
        assert!(
            std::any::type_name::<super::SkillReplacementSnapshot>()
                .ends_with("::SkillReplacementSnapshot")
        );
    }
}
