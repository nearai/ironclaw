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

// ── Budget invariants (issue #2843) ───────────────────────────
//
// Absolute ceilings on what any single budget configuration may allow,
// regardless of environment variables or user override. These are
// invariants: a `BudgetConfig` that tries to exceed any of them fails
// validation at load time. They exist so that a config bug cannot
// ruin a week of spend while the operator is asleep.

/// Absolute maximum wall-clock budget for any single thread (24 hours).
/// Raising this means an agent can run autonomously for more than a day —
/// review carefully.
pub const HARD_CAP_WALL_CLOCK_SECS: u64 = 86_400;

/// Absolute maximum iteration count backstop for any single thread.
/// Catches runaway loops that slip past the budget enforcer (e.g. a
/// cost-zero local LLM hitting a repeated-tool-call bug).
pub const HARD_CAP_ITERATIONS: usize = 10_000;

/// Absolute maximum USD budget for any single scope, as a string that
/// parses to `rust_decimal::Decimal`. A consumer should parse this once
/// and compare its own limits against the result. String form avoids a
/// `rust_decimal` dep on this base crate.
pub const HARD_CAP_BUDGET_USD_STR: &str = "100.00";
