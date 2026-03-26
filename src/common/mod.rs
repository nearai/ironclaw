//! Shared types and utilities for the IronClaw workspace.

mod event;
mod util;

pub use event::{AppEvent, ToolDecisionDto};
pub use util::truncate_preview;
