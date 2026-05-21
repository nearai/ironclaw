//! First-party userland extensions for IronClaw.
//!
//! This crate owns in-process extensions that ship with IronClaw but are not
//! kernel/runtime authority. Extensions receive explicit scoped handles and
//! export narrow ports back to Reborn composition.
#![forbid(unsafe_code)]

mod loaded;
mod skills;

pub use loaded::LoadedFirstPartyExtensions;
pub use skills::{
    FirstPartySkillsExtension, FirstPartySkillsExtensionError, FirstPartySkillsExtensionHandles,
};
