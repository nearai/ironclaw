# ironclaw_mcp guardrails

**Gate-pinned:** `tests/module_charter.rs` reads this file and requires it to
keep naming the failure-string rule and the gate; edit it with
`cargo test -p ironclaw_mcp` in hand.

- **Where new code goes is a charter question, answered in `src/lib.rs`.** The
  crate is seven private modules — `contract`, `runtime`, `client`, `jsonrpc`,
  `discovery`, `egress`, `diagnostics` — and the module-charter table in the
  `lib.rs` doc comment says what each owns and what must never drift into it
  (PROPOSAL §6.6.3). Read it before adding a file or a function. Two rules it
  carries are load-bearing here: **no module builds a failure string of its
  own** (every reason comes from `diagnostics`' cause enums, so the
  model-visible token set stays enumerable in one file), and **`discovery` owns
  the catalog rules while `client` owns the paging loop** (both read the same
  constants, so the two enforcement points cannot drift).
- **The failure-string rule is armed, and its carve-out is enumerated.**
  `tests/module_charter.rs` fails on any new `reason: "…"` / `reason: format!(…)`
  outside `diagnostics.rs`, and on a re-added `impl From<String> for
  McpClientError` (the implicit bypass — any `?` could mint a model-visible
  reason). The only grandfathered inline reasons are `runtime.rs`'s two
  `McpError` descriptor/invocation strings, which echo the manifest's own ids
  instead of classifying a failure; they are rows in that test, not a wildcard.
  Classify in your module, name in `diagnostics`.
- The submodules are private and every public item is re-exported from
  `lib.rs`, so `ironclaw_mcp::X` stays the single import path for consumers and
  a module rename is never a breaking change.
- Own the Reborn MCP runtime lane: MCP execution request/result types, client abstraction, host-mediated HTTP adapter, JSON-RPC exchange logic, and MCP-specific resource accounting.
- HTTP/SSE transports must go through host-mediated runtime egress. Do not add direct outbound networking, ad-hoc HTTP clients, DNS checks, credential injection, or network policy evaluation here.
- Treat plugin/runtime input as untrusted. Inputs may shape JSON-RPC arguments only; network policy, credentials, timeouts, and body limits must come from host-owned planning/handoff data.
- Preserve session isolation by scope/provider/url and keep session ids validated before reuse.
- Resource reservations supplied by host/runtime dispatch must be reconciled or released exactly once; do not create secondary reservations when a prepared reservation is present.
- **No budget authority (#7067).** The lane takes `ironclaw_host_api::resource::RuntimeResourceBudget` — reserve / reconcile / release, and nothing else — never `ResourceGovernor`. The kernel implements the port over its governor (`ironclaw_resources::GovernorRuntimeBudget`), so the lane cannot set limits, read account state, or name an account. `ironclaw_resources` is a **dev**-dependency only (the lane suites drive the port over the real governor); do not re-add it under `[dependencies]`, and do not widen the port.
- Surface only stable, sanitized client/runtime error categories. Do not expose upstream URLs with secrets, raw credentials, response bodies, or transport internals in runtime-visible errors.
- Keep MCP protocol concerns here; extension discovery belongs in `ironclaw_extension_registry`, network enforcement in `ironclaw_network`/host runtime egress, and product workflow outside this crate.

## See also

[`src/lib.rs`](./src/lib.rs) — the seven-module charter table (the
authoritative "where does new code go" answer). [`README.md`](./README.md) —
orientation: public surface, measured edges, tests.
[`../AGENTS.md`](../AGENTS.md) — the `lanes/` family boundary and its gates.
Contracts of record: `docs/reborn/contracts/mcp.md`,
`docs/reborn/contracts/runtime-workflows.md`,
`docs/reborn/contracts/processes.md`.
