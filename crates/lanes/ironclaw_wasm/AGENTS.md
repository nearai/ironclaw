# ironclaw_wasm

Owns the Reborn WASM component runtime lane.

**Gate-pinned:** `wasm_sandbox_core_module_stays_domain_free_v1_parity_kernel`
(`crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs`)
reads this file and requires it to keep the `wasm_sandbox_core` domain-free
wording below; edit it with `cargo test -p ironclaw_architecture_tests` in
hand.

## Responsibilities

- Load, compile, validate, meter, and execute already-selected WASM components for Reborn.
- Own and use the canonical WIT/component-model ABI in this crate's own `wit/` directory (`wit/tool.wit` and `wit/channel.wit`, both present today; `src/bindings.rs` reaches `wit/tool.wit` relative to the crate root, the wit-bindgen default). The directory moved here from the repo root under CHECKLIST WS4 / PROPOSAL §6.6.1: this crate is the ABI's owner, so the files live inside it and travel with it through the WS7 family move.
  - Every consumer of the *text* of `wit/tool.wit` reads `ironclaw_wasm::TOOL_WIT` rather than writing its own `include_str!`. The crate holds the single `include_str!`; `ironclaw_host_runtime` (which builds component fixtures the same way) uses the const over its existing cargo edge. Adding a second `include_str!` that reaches into this directory from another crate re-creates the §11.2.7 cross-crate reach-in the move removed.
  - `wit/tool.wit` (`near:agent@0.4.1`) and `wit/channel.wit` (`near:agent@0.3.1`) are the *same* WIT package name at two versions, so the directory cannot be handed to bindgen as a directory — always name the single file.
  - `scripts/check-version-bumps.sh` keys the ABI version gate off these two exact paths, and `WIT_TOOL_VERSION` in `src/config.rs` must equal `wit/tool.wit`'s package version.
  - `near:agent@0.4.1` is the sole supported tool contract. Its `tool.wit` ABI uses a typed `wit-result`-shaped response (`WitToolOutcome::Success`/`Failure`) instead of the retired `invoke-json` blob ABI. Components targeting another WIT contract version fail closed during instantiation with an unsupported-contract error.
- Provide thin host-import adapters for workspace, time, logging, secret-existence checks, tool invocation, and HTTP egress.
- Provide the folded `wasm_sandbox_core` module for domain-free Wasmtime/WASI sandbox primitives shared by runtime crates.
- Fail closed by default for host capabilities that are not explicitly wired by the Reborn composition root.

## Non-responsibilities

- Do not decide which tools/channels are exposed to the LLM.
- Do not own authorization, approvals, trust policy, dispatcher routing, run-state, or `CapabilityHost` orchestration.
- Do not perform direct production HTTP or secret retrieval; route those through injected host seams. Production HTTP egress belongs to the shared runtime egress service tracked by #3085.
- The V1 `src/tools/wasm/*` and `src/channels/wasm/*` trees are gone with the monolith; there is no compatibility reference to consult or depend on.
- Do not put ProductAdapter, tool, channel, workflow, dispatcher, secret, network, filesystem, host-runtime, or app composition dependencies in `wasm_sandbox_core`.

## Safety rules

- No JSON pointer/length ABI (`invoke_json`, `alloc`, `output_ptr`, `output_len`) in Reborn WASM.
- Instantiate fresh component instances per call.
- Preserve fuel, epoch timeout, aggregate memory, and table/instance limits; multi-memory components must not multiply the per-execution `memory_bytes` budget.
- Cap HTTP host-call timeouts to the remaining execution deadline, and require injected synchronous host implementations to honor that timeout.
- `ResourceUsage.network_egress_bytes` counts outbound request body bytes only; response-size limits are separate.
- Preserve usage/log snapshots on execution failure so sent egress can still be reconciled.

## See also

[`README.md`](./README.md) — orientation: public surface, measured edges, the
`wit/` ownership story, tests. [`../AGENTS.md`](../AGENTS.md) — the `lanes/`
family boundary and its gates. Contracts of record:
`docs/internal/reborn/contracts/wasm.md`, `docs/internal/reborn/contracts/runtime-workflows.md`,
`docs/internal/reborn/contracts/network.md`.
