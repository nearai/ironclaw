//! Skill types, parsing, selection, learning, and management for IronClaw.
//!
//! Skills are SKILL.md files (YAML frontmatter + markdown prompt) that extend the
//! agent's behavior through prompt-level instructions. This is a `substrates`-layer
//! domain crate: pure skill logic over `ironclaw_filesystem` +
//! `ironclaw_host_api`, with no runtime, loop, or product dependency.
//!
//! # Modules
//!
//! - [`types`] — manifests, activation criteria, trust levels, loaded skills.
//! - [`parser`](self) (private; re-exported) — the SKILL.md parser
//!   ([`parse_skill_md`]) for the OpenClaw skill format.
//! - [`selector`](self) (private; re-exported) — the *deterministic* prefilter for
//!   two-phase selection: no LLM involvement and no skill content in context, so
//!   a skill cannot influence its own selection.
//! - [`management`] / [`scoped_management`] — install / list / read / remove /
//!   search / update, over the raw filesystem and over a mount-scoped port.
//! - [`install_metadata`] — the on-disk record written for an installed skill.
//! - [`learning`] — distilling a reusable SKILL.md out of a completed run's
//!   transcript. Pure domain logic: inference sits behind `SkillInferencePort`,
//!   and the result is validated with the same parser install uses.
//! - [`validation`] — name validation, path-pattern checks, content escaping.
//!
//! # Trust model
//!
//! [`SkillTrust`] has two states, and the ordering (`Installed < Trusted`) is
//! load-bearing:
//!
//! - **Trusted** — user-placed skills (local / workspace).
//! - **Installed** — registry / external skills.
//!
//! **What trust gates is content exposure, not tool access.** The consuming side
//! is `ironclaw_loop_contracts::skill_context::SkillTrustLevel` (which mirrors
//! this enum deliberately, rather than depending on this crate), and it decides
//! whether the model sees a skill's prompt body or only its safe description.
//! Tool authority is a separate, unrelated mechanism owned by
//! `ironclaw_authorization` / `ironclaw_capabilities`; nothing in this crate
//! filters tools.

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
    SKILL_FILE_NAME, SkillContentRequest, SkillContentResult, SkillInstallFile,
    SkillInstallRequest, SkillInstallResult, SkillInstallSource, SkillManagementContext,
    SkillManagementError, SkillManagementErrorKind, SkillRemoveRequest, SkillRemoveResult,
    SkillSearchRequest, SkillSearchResult, SkillSource as ManagedSkillSource, SkillSummary,
    SkillUpdateRequest, SkillUpdateResult, install_skill, list_skills,
    normalize_install_bundle_relative_path, read_skill_content, remove_skill, runnable_skill_dir,
    search_skills, skill_summary_json, update_skill,
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
    SafeRelativePathError, escape_skill_content, escape_xml_attr, lint_skill_routing_metadata,
    lint_skill_routing_metadata_advisory, lint_skill_routing_metadata_blocking,
    normalize_line_endings, normalize_safe_relative_path, validate_credential_name,
    validate_credential_spec, validate_path_pattern, validate_skill_name,
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
