# ironclaw_wasm

The WASM component execution lane: load, compile, validate, meter, and execute
an already-selected WASM component under deny-by-default host imports, with a
fresh store per call and fuel/epoch/memory/table/instance ceilings. It also
owns the canonical tool/channel component-model ABI in its crate-local
[`wit/`](./wit) directory, and the folded `wasm_sandbox_core` module of
domain-free Wasmtime/WASI sandbox primitives. No other crate in the workspace
needs a WASM engine, and this lane (with its sibling limiter) is the only one
permitted to hold one.

- **Family / layer:** `lanes` / `runtimes` · **Package:** `ironclaw_wasm` ·
  **Manifest:** `crates/lanes/ironclaw_wasm/Cargo.toml`
- **Use this when:** an already-authorized invocation targets a WASM component,
  or a host needs the domain-free sandbox primitives (`wasm_sandbox_core`).
- **Don't use this when:** you're choosing which tools/channels the model
  sees → kernel/capability dispatch; you need resource ceilings for another
  wasmtime host → `ironclaw_wasm_limiter`; you need the ABI *text* → read
  `ironclaw_wasm::TOOL_WIT`, never a fresh `include_str!` into this crate's
  `wit/`.

## Public surface

- Runtime: `WitToolRuntime`, `WitToolHost`, `WitToolRequest`/
  `WitToolExecution`/`PreparedWitTool`, `WitToolRuntimeConfig`,
  `WIT_TOOL_VERSION`, `TOOL_WIT`; errors `WasmError`/`WasmHostError`.
- Host-import traits, each deny-by-default: `WasmHostHttp`,
  `WasmHostWorkspace`, `WasmHostSecrets`, `WasmHostTools`, `WasmHostClock`
  (with `Deny*`/`Recording*`/`System*` implementations — not a full matrix;
  re-derive with `rg -n "pub struct (Deny|Recording|System)" src/`), plus
  staged credential handoff (`WasmStagedRuntimeCredential(s)`,
  `WasmRuntimeCredentialProvider`).
- `wasm_sandbox_core` — engine setup, epoch ticker, minimal WASI p2 linker,
  limits, store-core helpers; deliberately domain-free.
- The ABI: `wit/tool.wit` (`near:agent@0.4.1`, typed `wit-result`-shaped
  `WitToolOutcome::Success`/`Failure` responses) is the sole supported tool
  contract. `wit/channel.wit` remains `near:agent@0.3.1`; the same WIT package
  name at two versions means bindgen is always handed the single file, never
  the directory. Components targeting another tool contract version fail
  closed during instantiation.

## Depends on / consumed by

- **Depends on (workspace, normal):** `ironclaw_host_api`,
  `ironclaw_wasm_limiter` — that second edge is the family's one
  runtimes→runtimes edge, pinned in `reborn_same_layer_edge_inventory.rs`.
  `ironclaw_extension_contracts` is dev-only today (measured 2026-08-05;
  `families/lanes.md` lists it as a normal dep — the tree is narrower than the
  design record here). External: `wasmtime`, `wasmtime-wasi`.
- **Consumed by (measured 2026-08-05):** `ironclaw_host_runtime` (normal);
  `ironclaw_integration_tests` dev-only. The nine `wasm-src/` guest components
  reach `wit/` by relative path — moving this crate rewrites all nine
  `wit-bindgen` `path:` args and forces a guest rebuild
  (`scripts/ci/check-wasm-artifact-freshness.py`).

## Invariants

- **Deny-by-default host imports:** a component gets exactly the host
  capabilities composition explicitly wires, nothing by omission.
- **Fresh store per call**, aggregate memory accounting across multi-memory
  components, fuel/epoch/table/instance ceilings (via the shared limiter).
- **Guest diagnostic safety:** each guest-authored failure code and message is
  scrubbed and bounded to 4 KiB at the sandbox-exit boundary without splitting
  UTF-8, and each buffered guest log record has the same bound. The host-runtime diagnostic seam applies the
  canonical `MODEL_DIAGNOSTIC_MAX_BYTES` bound again before tracing; typed
  provider messages are narrowed further before they enter dispatch metadata.
  Structured JSON envelopes retain their parseable kind/code/message shape.
- **`wasm_sandbox_core` stays domain-free** — no product, capability,
  registry, filesystem, network, secrets, host-runtime, or composition
  references; scanned by
  `wasm_sandbox_core_module_stays_domain_free_v1_parity_kernel`
  (`reborn_dependency_boundaries.rs`), which also pins two literal phrases in
  this crate's `AGENTS.md` — keep that file's wording intact.
- **No direct networking:** scanned by
  `reborn_runtime_http_egress_has_single_network_boundary`.
- **Single `include_str!` owner for the ABI text**, exported as `TOOL_WIT`;
  `scripts/check-version-bumps.sh` keys the ABI version gate off the two
  `wit/` paths, and `WIT_TOOL_VERSION` must equal `wit/tool.wit`'s package
  version.

## Tests

```bash
cargo test -p ironclaw_wasm
cargo test -p ironclaw_architecture_tests   # sandbox-core scan + egress scan + edge inventory
```

## See also

Working rules and safety rules: [`AGENTS.md`](./AGENTS.md) (canonical —
gate-pinned wording). Family boundary:
[`crates/lanes/AGENTS.md`](../AGENTS.md), including why `wit/` living here is
load-bearing. Contracts: `docs/internal/reborn/contracts/wasm.md`,
`docs/internal/reborn/contracts/runtime-workflows.md`,
`docs/internal/reborn/contracts/network.md`. Design record: PROPOSAL §6.6.1.
