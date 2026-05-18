# Agent Map — ironclaw_telegram_v2_adapter

## Start Here

- No crate-local CLAUDE.md exists yet; use this map plus `Cargo.toml` and source files.
- Read `src/lib.rs` first, then:
  - `adapter.rs` — ProductAdapter implementation.
  - `payload.rs` — Telegram payload parsing/DTO handling.
  - `render.rs` — Telegram outbound rendering.
- Read upstream contracts before changing adapter behavior:
  - `crates/ironclaw_product_adapters/AGENTS.md`
  - `crates/ironclaw_wasm_product_adapters/CLAUDE.md`

## What This Crate Owns

- Telegram WASM v2 ProductAdapter tracer-bullet implementation.
- Telegram payload parsing and outbound rendering for the adapter contract.
- Adapter-specific mapping between Telegram shapes and shared ProductAdapter DTOs.

## Do Not Move In Here

- Shared ProductAdapter contracts, registry semantics, or product workflow orchestration.
- Host auth minting, canonical conversation/thread binding, or turn coordination.
- Network egress, webhook listener setup, or secret storage.

## Validation

- Fast local check: `cargo test -p ironclaw_telegram_v2_adapter`
- Run `cargo test -p ironclaw_product_adapters` when shared DTO assumptions change.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Keep Telegram-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve adapter outputs as untrusted parsed DTOs until host/workflow stamps trusted context.
- Add tests before widening supported Telegram payload forms.
