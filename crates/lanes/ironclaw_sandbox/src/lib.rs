//! The sandboxed-process lane for IronClaw Reborn.
//!
//! One home for the three halves of "run an already-authorized command away
//! from the host", merged from `ironclaw_process_sandbox`,
//! `ironclaw_host_runtime::sandbox_process`, and `ironclaw_scripts`
//! (PROPOSAL §6.6.4):
//!
//! - [`plan`] — the typed [`SandboxProcessPlan`] contract. The kernel validates
//!   model-supplied plans through [`ValidatedSandboxProcessPlan`] before
//!   dispatching them under
//!   [`ironclaw_host_api::capability::PROCESS_SANDBOX_CAPABILITY_ID`].
//! - [`sandbox_process`] — the Docker/broker/credential-firewall/CA execution
//!   machinery behind [`ironclaw_host_api::process::SandboxCommandTransport`].
//! - [`script`] — the script lane and its Docker execution path.
//!
//! **Why one crate:** the `bollard`/`rcgen`/`libc` cone stays isolated in the
//! runtimes layer instead of sitting in the kernel, and the W6 egress-proxy /
//! sandbox work has a single owner.
//!
//! **Never:** ambient credentials (the credential-firewall design stays), and
//! no direct process spawning outside the transport seam.
//!
//! ## Wiring status
//!
//! Three production call paths cross this crate today, and none of them is
//! execution. Only the first is *plan validation*: `host_runtime`'s spawn path
//! parses and validates `SandboxProcessPlan`; `loop_host` compares against the
//! capability id; and `host_runtime::process_output` derives the scoped
//! saved-output directory through `RebornSandboxScopeKey`'s digest.
//! There is still no production backend for
//! `system.process_sandbox.run` — the Docker/CA machinery and the script lane
//! have no production constructor. See this crate's `CLAUDE.md`.

pub mod plan;
pub mod sandbox_process;
pub mod script;
mod validation;

#[cfg(test)]
mod plan_tests;

pub use plan::{
    ProcessSandboxPlanError, SandboxCommandPlan, SandboxCredentialBinding, SandboxInstallPlan,
    SandboxMount, SandboxMounts, SandboxNetworkPlan, SandboxProcessPlan,
    ValidatedSandboxProcessPlan,
};

pub use sandbox_process::{
    DEFAULT_SANDBOX_ALLOWED_DOMAINS, DEFAULT_SANDBOX_MAX_EGRESS_BYTES, RailwayPreviewSandboxConfig,
    RailwayPreviewSandboxTransport, RebornSandboxConfig, RebornSandboxContainerIdentity,
    RebornSandboxNetworkBroker, RebornSandboxScopeKey, RebornSandboxSecretBroker,
    RebornSandboxUserKey, RebornSandboxWorkspaceMode, RebornScopedSandboxCommandTransport,
    SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV, SANDBOX_MAX_EGRESS_BYTES_ENV, SandboxActivityRegistry,
    SandboxDockerReadiness, connect_docker_with_retry, sandbox_allowed_domains,
    sandbox_docker_readiness, sandbox_extra_allowed_domains, sandbox_max_egress_bytes,
    sandbox_network_policy,
};

pub use script::{
    DockerScriptBackend, ScriptBackend, ScriptBackendOutput, ScriptBackendRequest, ScriptError,
    ScriptExecutionRequest, ScriptExecutionResult, ScriptExecutor, ScriptHostHttpError,
    ScriptHostHttpResponse, ScriptInvocation, ScriptRuntime, ScriptRuntimeConfig,
    ScriptRuntimeHttpAdapter,
};

pub const DEFAULT_PROCESS_SANDBOX_IMAGE: &str = "ironclaw-process-sandbox:dev";
pub const DEFAULT_WORKSPACE_MOUNT: &str = "/workspace";
pub const DEFAULT_TOOLS_MOUNT: &str = "/ironclaw/state/tools";
pub const DEFAULT_CACHE_MOUNT: &str = "/ironclaw/state/cache";

pub(crate) const MAX_OUTPUT_LIMIT: u64 = 10 * 1024 * 1024;
pub(crate) const MAX_TIMEOUT_MS: u64 = 300_000;
