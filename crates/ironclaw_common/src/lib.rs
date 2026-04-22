//! Shared types and utilities for the IronClaw workspace.

mod event;
mod identity;
mod timezone;
mod util;

pub use event::{
    AppEvent, CodeExecutionFailureCategory, JobResultStatus, JobResultStatusParseError,
    OnboardingStateDto, PlanStepDto, SelfImprovementPhase, ToolDecisionDto,
};
pub use identity::{
    CredentialName, ExtensionName, ExternalThreadId, ExternalThreadIdError, IdentityError,
    MAX_EXTERNAL_THREAD_ID_LEN, MAX_MCP_SERVER_NAME_LEN, MAX_NAME_LEN, McpServerName,
    McpServerNameError,
};
pub use timezone::{ValidTimezone, deserialize_option_lenient};
pub use util::truncate_preview;

/// Maximum worker agent loop iterations. Used by the orchestrator (server-side
/// clamp in `create_job_inner`) and the worker runtime (`worker/job.rs`).
/// A single source of truth prevents the two from drifting.
pub const MAX_WORKER_ITERATIONS: u32 = 500;
