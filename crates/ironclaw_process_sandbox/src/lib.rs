//! Docker process sandbox process executor for IronClaw Reborn.
//!
//! This crate owns the dynamic process compatibility lane: a trusted host can
//! execute a typed [`SandboxProcessPlan`] through [`ProcessExecutor`] while
//! keeping host paths in executor configuration and secret material behind
//! broker policy.

mod approval;
mod backend;
mod broker;
mod docker;
mod plan;
mod validation;

pub use approval::{
    SandboxApprovalCredential, SandboxApprovalMount, SandboxProcessApprovalSummary,
};
pub use backend::{
    ProcessSandboxBackend, ProcessSandboxError, ProcessSandboxExecutor, SandboxPhaseOutput,
    SandboxProcessOutput, SandboxProcessRequest, SandboxProcessResult,
};
pub use broker::{BrokerHeaderRewrite, BrokerRewriteResult, SandboxBrokerPolicy};
pub use docker::{
    DockerBrokerConfig, DockerInvocation, DockerProcessSandboxBackend, DockerProcessSandboxConfig,
    DockerRunError, DockerRunOutput, DockerRunner, SandboxProcessPhase, SystemDockerRunner,
    docker_invocation_for_phase,
};
pub use plan::{
    SandboxCommandPlan, SandboxCredentialBinding, SandboxInstallPlan, SandboxMount, SandboxMounts,
    SandboxNetworkPlan, SandboxPlanError, SandboxProcessPlan, ValidatedSandboxProcessPlan,
};

pub const DEFAULT_PROCESS_SANDBOX_IMAGE: &str = "ironclaw-process-sandbox:dev";
pub const PROCESS_SANDBOX_CAPABILITY_ID: &str = "system.process_sandbox.run";
pub const DEFAULT_WORKSPACE_MOUNT: &str = "/workspace";
pub const DEFAULT_TOOLS_MOUNT: &str = "/ironclaw/state/tools";
pub const DEFAULT_CACHE_MOUNT: &str = "/ironclaw/state/cache";

pub(crate) const DEFAULT_STDOUT_LIMIT: u64 = 1024 * 1024;
pub(crate) const DEFAULT_STDERR_LIMIT: u64 = 256 * 1024;
pub(crate) const DEFAULT_TIMEOUT_MS: u64 = 30_000;
