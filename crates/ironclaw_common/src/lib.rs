//! Shared types and utilities for the IronClaw workspace.

mod event;
mod identity;
mod timezone;
mod util;

pub use event::{
    AppEvent, JobResultStatus, JobResultStatusParseError, OnboardingStateDto, PlanStepDto,
    ToolDecisionDto,
};
pub use identity::{
    CredentialName, ExtensionName, GitHubRepo, GitHubRepoError, IdentityError, MAX_NAME_LEN,
};
pub use timezone::{ValidTimezone, deserialize_option_lenient};
pub use util::truncate_preview;

/// Maximum worker agent loop iterations. Used by the orchestrator (server-side
/// clamp in `create_job_inner`) and the worker runtime (`worker/job.rs`).
/// A single source of truth prevents the two from drifting.
pub const MAX_WORKER_ITERATIONS: u32 = 500;
