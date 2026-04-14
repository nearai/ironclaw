//! Shared types and utilities for the IronClaw workspace.

pub mod capabilities;
pub mod capabilities_schema;
pub mod credential_mapping;
mod event;
pub mod ext_error;
pub mod limits;
pub mod oauth_refresh;
pub mod rate_limit;
pub mod storage;
mod timezone;
mod util;

pub use event::{AppEvent, PlanStepDto, ToolDecisionDto};
pub use timezone::{ValidTimezone, deserialize_option_lenient};
pub use util::truncate_preview;

/// Maximum worker agent loop iterations. Used by the orchestrator (server-side
/// clamp in `create_job_inner`) and the worker runtime (`worker/job.rs`).
/// A single source of truth prevents the two from drifting.
pub const MAX_WORKER_ITERATIONS: u32 = 500;
