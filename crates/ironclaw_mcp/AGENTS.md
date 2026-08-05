# Agent Map — ironclaw_mcp

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these Reborn contracts as the source of truth before changing behavior:
- `docs/reborn/contracts/mcp.md`
- `docs/reborn/contracts/runtime-workflows.md`
- `docs/reborn/contracts/processes.md`

## Module Charter

The crate is **seven private modules**, each with a stated owner; `lib.rs`
carries the charter table (PROPOSAL §6.6.3) and re-exports every public item,
so `ironclaw_mcp::X` remains the only import path. Consult the table in
`src/lib.rs` before adding a file:

| Module | Owns |
|---|---|
| `contract` | The vocabulary a caller names: config, DTOs, the `McpClient`/`McpExecutor` traits, the `McpError`/`McpClientError` taxonomy |
| `runtime` | Resource-governed execution: descriptor admission, reserve → call → reconcile/release, the manifest credential context |
| `client` | The Streamable-HTTP `McpClient`: handshake, per-invocation session lifecycle, the `tools/list` paging loop |
| `jsonrpc` | The JSON-RPC 2.0 codec and response hygiene: framing, id matching, session id / protocol version, auth challenge, per-method credential routing |
| `discovery` | `tools/list` catalog admission: host ceilings, per-tool classification, schema bounds, tool-name grammar |
| `egress` | The host-mediated HTTP seam: the `McpHostHttp` port and the host-owned egress plan/planner |
| `diagnostics` | Every stable, bounded failure token the lane surfaces |

## What This Crate Owns

- The Reborn MCP runtime lane (fail-closed process policy, host-mediated egress), currently:
- Runtime + executor: `McpRuntime`, the `McpExecutor` trait, and `McpRuntimeConfig`.
- Execution request/result types: `McpInvocation`, `McpExecutionRequest`, `McpExecutionResult` (result field is the shared `ironclaw_host_api::resource::CapabilityHostResult`); `McpError`.
- Client abstraction: the `McpClient` trait with `McpClientRequest` / `McpClientOutput` (JSON-RPC exchange).
- Host-mediated HTTP: the `McpHostHttp` trait, `McpRuntimeHttpAdapter`, `McpHostHttpClient`, the egress planner (`McpHostHttpEgressPlanner` / `StaticMcpHostHttpEgressPlanner`, `McpHostHttpEgressPlan`/`McpHostHttpEgressPlanRequest`), and the shared `ironclaw_host_api::http::CapabilityHostHttpRequest` / `McpHostHttpResponse` / `McpHostHttpError`.
- Crate-local public API, tests, and fixtures needed to prove that ownership.

## Do Not Move In Here

- direct process starts, manual credentials, or direct network egress outside mediated substrates.
- Secrets, raw host paths, backend error details, and unredacted user content in errors, events, snapshots, logs, or docs.

## Validation

- Fast local check: `cargo test -p ironclaw_mcp`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`
- If production persistence behavior changes, add/maintain PostgreSQL and libSQL parity tests.

## Agent Notes

- Keep edits inside this crate unless a contract explicitly requires a neighboring crate change.
- Prefer caller-level tests when a helper gates dispatch, persistence, network, secrets, approvals, resources, events, or process side effects.
- If the contract and code disagree, stop and treat the task as a contract-change request instead of silently changing ownership.
