//! Skills system for IronClaw.
//!
//! This module re-exports everything from the `ironclaw_skills` crate,
//! keeping `crate::skills::*` imports working throughout the codebase.
//! New code should import from `ironclaw_skills` directly.
//!
//! The `attenuation` submodule remains here because it depends on
//! `crate::llm::ToolDefinition` which is a main-crate type.

pub mod attenuation;

// Re-export everything from the extracted crate.
pub use ironclaw_skills::*;

// Re-export attenuation at the same path as before.
pub use attenuation::{AttenuationResult, attenuate_tools};
