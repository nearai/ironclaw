//! Skill activation, selection, context and execution adapters.
//!
//! ## Where this came from
//!
//! ✎ **WS8, 2026-08-05.** These modules were the crate
//! `ironclaw_first_party_extension_ports` (PROPOSAL §9 row 55,
//! "delete-after-migration (dissolve)"). That crate existed for exactly one
//! reason — to break the `first_party_extensions → loop_host → host_runtime →
//! first_party_extensions` cycle — and the cycle's last edge went with WS3's
//! kernel narrowing, so the crate had nothing left to be. Measured at the
//! dissolution: every dependency it declared (`host_api`, `loop_contracts`,
//! `filesystem`, `skills`, `turns`, plus `async-trait`/`futures`/`serde_json`/
//! `thiserror`/`tracing`) was already a dependency of `ironclaw_loop_host`, so
//! folding it in added **zero** dependency edges, and both crates already sat
//! at layer `loops`.
//!
//! §9's row named three destinations — activation machinery to
//! `loop_host`/`skills`, observer vocabulary to `skills`, the bundle asset
//! reader to a package. Two of the three are **not reachable** and the row is
//! corrected rather than half-executed: `SkillActivationObservedEvent` carries
//! `LoopRunContext` and `SkillActivationRequest`, and the latter carries
//! `SkillBundleId`/`SkillSourceKind` — all owned above `ironclaw_skills`, whose
//! only dependencies are `filesystem` and `host_api`. Moving the observer
//! vocabulary there would need `skills → loop_host`, which is a cycle
//! (`loop_host → skills` is live). The asset reader is bound to the same
//! `loop_host` bundle vocabulary, and `ironclaw_extension_support` sits at
//! layer `runtimes`, *below* `loops`, so it cannot reach it either. The
//! primary destination §9 named — `loop_host` — takes all of it.
//!
//! ## What lives here
//!
//! Loop-facing adapters over the crate's own skill-bundle vocabulary
//! (`skill_bundle_source`, `skill_bundle_context_source`, `skill_context`):
//! activation selection and its observer seam, the `ironclaw.skill.activate`
//! capability, the bundle asset reader, and the scoped handles granted to
//! bundled skill-context implementations. Concrete tool behavior stays in the
//! userland implementation crates below.

mod activation;
mod assets;
mod bundle_staging;
mod error;
mod execution;
mod setup_markers;
mod skill_activation_capability;
mod skills;

pub use activation::{
    DEFAULT_MAX_ACTIVE_SKILLS, DEFAULT_MAX_SKILL_CONTEXT_TOKENS, SelectableSkillContextSource,
    SkillActivationMode, SkillActivationObservedEvent, SkillActivationObserver,
    SkillActivationPlan, SkillActivationRequest, SkillActivationSelection,
    SkillActivationSelectionError, SkillActivationSelectionMode, SkillActivationSelectorConfig,
    SkillInjectionMode,
};
pub use assets::{SkillBundleAsset, SkillBundleAssetReadError, SkillBundleAssetReader};
pub use bundle_staging::{
    STAGED_SKILLS_DIRNAME, SkillBundleStager, StagedBundleFile, WorkspaceSkillBundleStager,
};
pub use error::FirstPartySkillsExtensionError;
pub use execution::{SkillExecutionAdapter, SkillExecutionAdapterError, SkillExecutionPlan};
pub use skill_activation_capability::{SKILL_ACTIVATE_CAPABILITY_ID, skill_activation_capability};
pub use skills::{
    FirstPartySelectableSkillsRuntime, FirstPartySkillsExtension, FirstPartySkillsExtensionHandles,
};
