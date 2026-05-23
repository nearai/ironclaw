//! First-party userland extensions for IronClaw.
//!
//! This crate owns in-process extensions that ship with IronClaw but are not
//! kernel/runtime authority. Extensions receive explicit scoped handles and
//! export narrow ports back to Reborn composition.
#![forbid(unsafe_code)]

mod activation;
mod assets;
mod builtin_tools;
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
pub use builtin_tools::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, BuiltinFirstPartyTools,
    ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID,
    JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, SHELL_CAPABILITY_ID,
    TIME_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID, builtin_first_party_handlers,
    builtin_first_party_package,
};
pub use error::FirstPartySkillsExtensionError;
pub use execution::{SkillExecutionAdapter, SkillExecutionAdapterError, SkillExecutionPlan};
pub use loaded::LoadedFirstPartyExtensions;
pub use skills::{FirstPartySkillsExtension, FirstPartySkillsExtensionHandles};
