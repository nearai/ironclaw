//! Core hook types and traits.
//!
//! All types are re-exported from the `ironclaw_hooks` crate, which is the
//! canonical source. This module exists so existing code can use
//! `crate::hooks::Hook` instead of knowing about `ironclaw_hooks` directly.

pub use ironclaw_hooks::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint,
};
