# Agent Map — ironclaw_wasm_product_adapters

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_product_adapters/AGENTS.md`
  - `crates/ironclaw_wasm/CLAUDE.md`
  - `crates/ironclaw_product_adapter_registry/AGENTS.md`
  - `docs/reborn/contracts/network.md`

## What This Crate Owns

- WASM v2 host runtime for ProductAdapter components.
- Adapter-specific host control-plane glue: protocol-auth verification, manifest egress preflight, component runtime, runner, bindings, store, config.
- ProductAdapter WIT/request shapes and parse/render-only component execution path.
- Seams for host-runtime egress injection; current HTTP egress import fails closed until wired.

## Do Not Move In Here

- Shared product-surface DTOs; keep those in `ironclaw_product_adapters`.
- Product workflow, canonical user/thread binding, turn coordination, outbound projection cursors, delivery-status persistence, runtime dispatch, process spawning, or authorization/approval policy.
- Production HTTP/DNS/private-IP checks/redirect handling/response limits/secret injection/leak scanning/redaction; delegate through host-runtime/network/secrets.
- Auth evidence fabrication by WASM components.

## Validation

- Fast local check: `cargo test -p ironclaw_wasm_product_adapters`
- Focused check: `cargo test -p ironclaw_wasm_product_adapters --test component_runtime_contract`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Host glue stamps trusted adapter/installation/auth/received context after WASM returns parsed DTOs.
- No-op/ignored authenticated events must be explicit parsed DTO payloads, not absent parse results.
- Preserve minimal WASI p2: clock/random allowed; env,args,stdio,preopens,inherited network,DNS disabled.
