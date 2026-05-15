//! Shared types and utilities used across the IronClaw workspace.
#![warn(unreachable_pub)]

mod attachment;
pub mod env_helpers;
mod event;
mod identity;
pub mod paths;
mod platform;
mod timezone;
#[allow(dead_code)] // Trust-boundary scaffolding for the Reborn architecture; not yet consumed.
mod trust_boundary;
mod util;

pub use attachment::{AttachmentKind, IncomingAttachment};
pub use event::{
    AppEvent, CodeExecutionFailureCategory, JobResultStatus, OnboardingStateDto, PlanStepDto,
    SelfImprovementPhase, ToolDecisionDto,
};
pub use identity::{
    CredentialName, ExtensionName, ExternalThreadId, ExternalThreadIdError,
    MAX_MCP_SERVER_NAME_LEN, MAX_NAME_LEN, McpServerName,
};
pub use paths::ironclaw_base_dir;
pub use platform::PlatformInfo;
pub use timezone::{ValidTimezone, deserialize_option_lenient};
pub use util::{truncate_for_preview, truncate_preview};

/// Maximum worker agent loop iterations. Used by the orchestrator (server-side
/// clamp in `create_job_inner`) and the worker runtime (`worker/job.rs`).
/// A single source of truth prevents the two from drifting.
pub const MAX_WORKER_ITERATIONS: u32 = 500;
