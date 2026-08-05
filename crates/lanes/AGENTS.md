# `crates/lanes/` — execution mechanisms

How an already-authorized invocation actually runs: the WASM component lane, the shared wasmtime resource limiter, the MCP lane, and the sandbox/process lane. A lane executes work the kernel already permitted; it never re-decides permission and never widens it.

## Members

| Crate | Layer | Charter |
| --- | --- | --- |
| [`ironclaw_mcp`](./ironclaw_mcp) | `runtimes` | the MCP lane (host-mediated HTTP only) |
| [`ironclaw_sandbox`](./ironclaw_sandbox) | `runtimes` | the sandbox/process lane: plan contract, Docker/broker/CA machinery, Docker script backend |
| [`ironclaw_wasm`](./ironclaw_wasm) | `runtimes` | the WASM component lane; owns the tool/channel ABI in its crate-local [`wit/`](./ironclaw_wasm/wit) |
| [`ironclaw_wasm_limiter`](./ironclaw_wasm_limiter) | `runtimes` | the shared wasmtime `ResourceLimiter` |

**`wit/` lives inside `ironclaw_wasm`, and that is load-bearing.** The tool and
channel ABIs are crate assets (PROPOSAL §6.6.1), so this family directory holds
no non-crate directory of its own — which is what §11.2.1's no-stray-toplevel
check exists to require. The cost is paid by the guests instead: every
`wasm-src/` component reaches the ABI by relative path, so moving this crate
rewrites nine `wit-bindgen` `path:` args and forces a rebuild of the six
committed `.wasm` binaries (`scripts/ci/check-wasm-artifact-freshness.py` keys
each artifact to a digest of its whole `wasm-src/` tree and forbids re-recording
without rebuilding). That is why the move shipped as its own PR (WS7 2/2), and
why the next one should not be undertaken casually.

## Rules that outrank this file

- **Full charter, boundaries, dependency direction, and security posture:** [`docs/reborn/target-architecture/families/lanes.md`](../../docs/reborn/target-architecture/families/lanes.md).
- **A family directory is never a compilation or trust unit.** The mechanically enforced dependency truth is each crate's `[package.metadata.ironclaw] layer`, checked by `crates/app/ironclaw_architecture_tests`. Family placement is ownership and discoverability only (PROPOSAL §5).
- **Moving a crate between families is not a rename.** A crate's directory carries its full package name; the family word never enters the crate name (PROPOSAL §5.1).
