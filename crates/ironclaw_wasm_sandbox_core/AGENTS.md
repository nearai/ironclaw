# Agent Map — ironclaw_wasm_sandbox_core

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` and `src/lib.rs` for exported primitives.
- Neighboring runtime crates:
  - `crates/ironclaw_wasm/AGENTS.md`
  - `crates/ironclaw_wasm_product_adapters/AGENTS.md`

## What This Crate Owns

- Domain-free Wasmtime/WASI sandbox kernel pieces.
- Component-engine setup, epoch ticker, minimal WASI p2 linker, resource limiter, limits, and store-core helpers.
- Reborn v1-style minimal WASI semantics and resource-limit primitives shared by runtime crates.

## Do Not Move In Here

- ProductAdapter, tool, channel, workflow, dispatcher, secret, network, filesystem, host-runtime, or app composition dependencies.
- Runtime-specific WIT bindings, host trait implementations, or custom host imports.
- HTTP, DNS/private-IP checks, secret injection, leak scanning, redaction, workspace reads, tool invocation, product workflow calls, or channel lifecycle logic.

## Validation

- Fast local check: `cargo test -p ironclaw_wasm_sandbox_core`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- Run downstream runtime tests when changing exported sandbox primitives: `cargo test -p ironclaw_wasm` and `cargo test -p ironclaw_wasm_product_adapters`.

## Agent Notes

- Preserve minimal WASI p2: clock/random allowed; env,args,stdio inheritance, preopened directories, inherited network, and DNS lookup disabled.
- Preserve fuel, epoch timeout, aggregate memory, table, instance, and memory limits per execution.
- Multi-memory components must not multiply the configured `memory_bytes` budget.
