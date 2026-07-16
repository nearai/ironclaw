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
- The runtime + execution surface: `WitToolRuntime` (`runtime`), `WitToolHost`, `WitToolRequest`/`WitToolExecution`/`PreparedWitTool`, `WasmLogLevel`/`WasmLogRecord` (`types`, `host`), and `bindings`/`store`.
- The folded `wasm_sandbox_core` module: domain-free Wasmtime/WASI sandbox primitives, component-engine setup, epoch ticker, minimal WASI p2 linker, resource limiter, limits, and store-core helpers.
- Runtime config + ABI version: `WitToolRuntimeConfig`, `WIT_TOOL_VERSION` (`config`; per-execution limits use `wasm_sandbox_core::SandboxLimits`); errors `WasmError`/`WasmHostError` (`error`).
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
- Keep `wasm_sandbox_core` domain-free: no ProductAdapter, tool, channel, workflow, dispatcher, secret, network, filesystem, host-runtime, or app composition dependencies.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
