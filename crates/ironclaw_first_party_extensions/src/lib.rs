//! First-party userland extensions for IronClaw.
//!
//! This crate owns in-process extensions that ship with IronClaw but are not
//! kernel/runtime authority. Extensions receive explicit scoped handles and
//! export narrow ports back to Reborn composition.
#![forbid(unsafe_code)]

mod activation;
mod assets;
mod error;
mod execution;
mod loaded;
mod skills;

pub use activation::{
    DEFAULT_MAX_ACTIVE_SKILLS, DEFAULT_MAX_SKILL_CONTEXT_TOKENS, SelectableSkillContextSource,
    SkillActivationMode, SkillActivationPlan, SkillActivationRequest, SkillActivationSelection,
    SkillActivationSelectionError, SkillActivationSelectorConfig,
};
pub use assets::{SkillBundleAsset, SkillBundleAssetReadError, SkillBundleAssetReader};
pub use error::FirstPartySkillsExtensionError;
pub use execution::{SkillExecutionAdapter, SkillExecutionAdapterError, SkillExecutionPlan};
pub use loaded::LoadedFirstPartyExtensions;
pub use skills::{FirstPartySkillsExtension, FirstPartySkillsExtensionHandles};
