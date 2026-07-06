# Agent Map — ironclaw_wasm

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/wasm.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/network.md`

## What This Crate Owns

- The Reborn WASM component runtime lane (load/compile/validate/meter/execute already-selected components), currently:
- The runtime + execution surface: `WitToolRuntime` (`runtime`), `WitToolHost`, `WitToolRequest`/`WitToolExecution`/`PreparedWitTool`, `WasmLogLevel`/`WasmLogRecord` (`types`, `host`), and `bindings`/`limiter`/`store`.
- Runtime config + ABI version: `WitToolRuntimeConfig`, `WIT_TOOL_VERSION` (`config`; per-execution limits use the shared `ironclaw_wasm_sandbox_core::SandboxLimits`); errors `WasmError`/`WasmHostError` (`error`).
- Mediated, fail-closed host-capability seams (`host`): the `WasmHostHttp`/`WasmHostSecrets`/`WasmHostTools`/`WasmHostWorkspace`/`WasmHostClock` traits with their `Deny*`/`Recording*`/`System*` implementations, `WasmRuntimeHttpAdapter` + `WasmHttpRequest`/`WasmHttpResponse`, and staged credential handoff (`WasmRuntimeCredentialProvider`/`WasmRuntimeCredentialRequest`, `WasmStagedRuntimeCredential`/`WasmStagedRuntimeCredentials`, `WasmRuntimePolicyDiscarder`).
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- privileged host effects outside mediated APIs or copied secrets/network/resource logic.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_wasm`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
