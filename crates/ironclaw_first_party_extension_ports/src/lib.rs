//! Loop-facing ports for first-party IronClaw extensions.
//!
//! This crate owns adapters that expose first-party extension behavior to the
//! loop and turn-run layers. Concrete tool behavior stays in lower userland
//! implementation crates.
#![forbid(unsafe_code)]

mod activation;
mod assets;
mod error;
mod execution;
mod setup_markers;
mod skills;

pub use activation::{
    DEFAULT_MAX_ACTIVE_SKILLS, DEFAULT_MAX_SKILL_CONTEXT_TOKENS, SelectableSkillContextSource,
    SkillActivationMode, SkillActivationObservedEvent, SkillActivationObserver,
    SkillActivationPlan, SkillActivationRequest, SkillActivationSelection,
    SkillActivationSelectionError, SkillActivationSelectorConfig,
};
pub use assets::{SkillBundleAsset, SkillBundleAssetReadError, SkillBundleAssetReader};
pub use error::FirstPartySkillsExtensionError;
pub use execution::{SkillExecutionAdapter, SkillExecutionAdapterError, SkillExecutionPlan};
pub use skills::{
    FirstPartySelectableSkillsRuntime, FirstPartySkillsExtension, FirstPartySkillsExtensionHandles,
};
