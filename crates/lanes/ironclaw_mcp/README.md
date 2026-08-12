# ironclaw_mcp

The MCP execution lane: adapts manifest-declared MCP tools into capabilities
over **host-mediated HTTP only**. The crate holds no HTTP client dependency at
all — every outbound JSON-RPC call is planned and executed through the injected
egress port, so the lane cannot originate a connection the kernel has not
mediated. It is a distinct protocol lane with its own discovery and JSON-RPC
surface; the per-lane external rule (a WASM engine only in the WASM lane,
container machinery only in the sandbox lane, no network stack here) stays
statable only while each lane is its own crate.

- **Family / layer:** `lanes` / `runtimes` · **Package:** `ironclaw_mcp` ·
  **Manifest:** `crates/lanes/ironclaw_mcp/Cargo.toml`
- **Use this when:** an already-authorized invocation targets an MCP server's
  tool, or composition is wiring the MCP runtime with a concrete host-mediated
  transport.
- **Don't use this when:** you're deciding *whether* the call is allowed →
  kernel; you need what a package *is* (manifest, install record) →
  `ironclaw_extension_contracts` vocabulary / the extensions family; you need
  raw HTTP → `ironclaw_network` behind the kernel's egress seam, never here.

## Public surface

Seven private modules with a charter table in the `src/lib.rs` doc comment;
every public item re-exports through `lib.rs`, so `ironclaw_mcp::X` is the
single import path and a module rename is never a breaking change. Highlights:

- Runtime: `McpRuntime`, `McpExecutor`, `McpRuntimeConfig`; requests/results
  `McpInvocation`, `McpExecutionRequest`, `McpExecutionResult`; `McpError`.
- Client seam: the `McpClient` trait with `McpClientRequest`/`McpClientOutput`
  (Streamable-HTTP JSON-RPC: handshake, per-invocation session lifecycle, the
  `tools/list` paging loop).
- Egress: the `McpHostHttp` port, `McpRuntimeHttpAdapter`, `McpHostHttpClient`,
  and the egress planner pair (`McpHostHttpEgressPlanner` /
  `StaticMcpHostHttpEgressPlanner`).
- Diagnostics: every stable, bounded failure token the lane surfaces, in one
  module.

## Depends on / consumed by

- **Depends on (workspace, normal):** `ironclaw_host_api`,
  `ironclaw_extension_contracts` — nothing else. `ironclaw_extension_registry`
  and `ironclaw_resources` are **dev-dependencies only** (the lane suites drive
  the budget port over the real governor); do not promote either.
- **Consumed by (measured 2026-08-05):** `ironclaw_host_runtime` (the lane
  executor), `ironclaw_extension_host`, `ironclaw_composition`.

## Invariants

- **Host-mediated HTTP only:** no `reqwest`/`hyper`/socket use anywhere in
  `src/` — scanned by `reborn_runtime_http_egress_has_single_network_boundary`
  (`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`),
  which reads this crate's whole `src/` tree with a non-vacuity assertion.
- **No module builds a failure string of its own:** reasons come from
  `diagnostics`' cause enums — armed by `tests/module_charter.rs` (which also
  pins that `AGENTS.md` keeps naming the rule and the gate, and keeps
  `impl From<String> for McpClientError` deleted).
- **No budget authority (#7067):** the lane takes
  `ironclaw_host_api::resource::RuntimeResourceBudget`
  (reserve/reconcile/release) — never `ResourceGovernor`; reservations
  reconcile or release exactly once.
- **Session isolation** by scope/provider/url; session ids validated before
  reuse (guardrails, `AGENTS.md`).

## Tests

```bash
cargo test -p ironclaw_mcp                  # includes tests/module_charter.rs
cargo test -p ironclaw_architecture_tests   # egress scan + layer matrix
```

## See also

Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guardrails —
gate-pinned by `tests/module_charter.rs`). Family boundary:
[`crates/lanes/AGENTS.md`](../AGENTS.md). Contracts:
`docs/reborn/contracts/mcp.md`, `docs/reborn/contracts/runtime-workflows.md`,
`docs/reborn/contracts/processes.md`. Design record: PROPOSAL §6.6.3.
