# Agent Map — ironclaw_sandbox

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/scripts.md`
- `docs/reborn/contracts/processes.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/network.md`

## What This Crate Owns

- The **sandboxed-process lane**: the typed `SandboxProcessPlan` contract, the
  Docker/broker/credential-firewall/CA execution machinery behind
  `ironclaw_host_api::process::SandboxCommandTransport`, and the script runtime
  lane over host-mediated filesystem/events/resources/dispatcher/HTTP:
- Runtime + executor: `ScriptRuntime`, the `ScriptExecutor` trait, and `ScriptRuntimeConfig`.
- Execution request/result types: `ScriptInvocation`, `ScriptExecutionRequest`, `ScriptExecutionResult` (result field is the shared `ironclaw_host_api::resource::CapabilityHostResult`); `ScriptError`.
- Backend abstraction: the `ScriptBackend` trait + `DockerScriptBackend`, with normalized `ScriptBackendRequest` / `ScriptBackendOutput` (output parsing).
- Host-mediated HTTP: `ScriptRuntimeHttpAdapter` and the shared `ironclaw_host_api::http::CapabilityHostHttpRequest` / `ScriptHostHttpResponse` / `ScriptHostHttpError`.
- Plan contract: `SandboxProcessPlan`, `ValidatedSandboxProcessPlan`, and the mount/network/credential plan types.
- Sandbox execution: `RebornScopedSandboxCommandTransport`, `RebornSandboxConfig`, the network broker/allowlist, credential firewall, container identity, sandbox CA, mounts, and scope/user keys.
- The `bollard`/`rcgen`/`libc` cone — declared by this crate and no other.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- manual credentials, direct provider HTTP, or duplicated dispatcher/process/resource policy.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_sandbox`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
