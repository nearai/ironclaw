//! Process sandbox plan types for IronClaw Reborn.
//!
//! This crate owns the typed [`SandboxProcessPlan`] contract: the runtime
//! validates model-supplied plans through [`ValidatedSandboxProcessPlan`]
//! before the host dispatches them under
//! [`PROCESS_SANDBOX_CAPABILITY_ID`]. There is no production backend wired
//! for that capability today (see host_runtime's `process_executor`); this
//! crate only owns plan validation.

mod plan;
mod validation;

pub use plan::{
    ProcessSandboxPlanError, SandboxCommandPlan, SandboxCredentialBinding, SandboxInstallPlan,
    SandboxMount, SandboxMounts, SandboxNetworkPlan, SandboxProcessPlan,
    ValidatedSandboxProcessPlan,
};

#[cfg(test)]
mod tests;

pub const DEFAULT_PROCESS_SANDBOX_IMAGE: &str = "ironclaw-process-sandbox:dev";
pub const PROCESS_SANDBOX_CAPABILITY_ID: &str = "system.process_sandbox.run";
pub const DEFAULT_WORKSPACE_MOUNT: &str = "/workspace";
pub const DEFAULT_TOOLS_MOUNT: &str = "/ironclaw/state/tools";
pub const DEFAULT_CACHE_MOUNT: &str = "/ironclaw/state/cache";

pub(crate) const MAX_OUTPUT_LIMIT: u64 = 10 * 1024 * 1024;
pub(crate) const MAX_TIMEOUT_MS: u64 = 300_000;
