//! Shared types and utilities for the IronClaw workspace.

mod event;
mod util;

pub use event::{AppEvent, ToolDecisionDto};
pub use util::truncate_preview;

/// Maximum worker agent loop iterations. Used by the orchestrator (server-side
/// clamp in `create_job_inner`) and the worker runtime (`worker/job.rs`).
/// A single source of truth prevents the two from drifting.
pub const MAX_WORKER_ITERATIONS: u32 = 500;
