# `crates/lanes/` — execution mechanisms

How an already-authorized invocation actually runs: the WASM component lane, the shared wasmtime resource limiter, the MCP lane, and the sandbox/process lane. A lane executes work the kernel already permitted; it never re-decides permission and never widens it.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_mcp`](./ironclaw_mcp) | `runtimes` | the MCP lane (host-mediated HTTP only) |
| [`ironclaw_sandbox`](./ironclaw_sandbox) | `runtimes` | the sandbox/process lane: plan contract, Docker/broker/CA machinery, Docker script backend |
| [`ironclaw_wasm_limiter`](./ironclaw_wasm_limiter) | `runtimes` | the shared wasmtime `ResourceLimiter` |

**Not here yet:** `ironclaw_wasm` (the WASM component lane, `runtimes`) is still at
`crates/ironclaw_wasm`. Its move carries `wit/` with it, which forces every
`wasm-src/` guest's `wit-bindgen` path and a rebuild of the committed `.wasm`
binaries, so it ships as its own PR (WS7 2/2) rather than riding the text-only
move.

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/lanes.md`](../../docs/reborn/target-architecture/families/lanes.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
