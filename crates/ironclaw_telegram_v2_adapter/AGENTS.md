# Agent Map — ironclaw_telegram_v2_adapter

## Start Here

- No crate-local CLAUDE.md exists yet; use this map plus `Cargo.toml` and source files.
- Read `src/lib.rs` first, then:
  - `payload.rs` — Telegram Bot API payload normalization/DTO handling.
  - `render.rs` — Telegram outbound request rendering.
  - There is no adapter in this crate; the `ChannelAdapter` lives in
    `ironclaw_telegram_extension`.
- Read upstream contracts before changing adapter behavior:
  - `crates/ironclaw_product/AGENTS.md`

## What This Crate Owns

- The Telegram Bot API **protocol engine**: pure payload normalization and
  outbound request rendering, with no I/O and no secrets. It is a plain native
  crate (no WASM target) and contains no adapter.
- Adapter-specific mapping between Telegram shapes and the shared channel DTOs.

## Do Not Move In Here

- Shared ProductAdapter contracts, registry semantics, or product workflow orchestration.
- Host auth minting, canonical conversation/thread binding, or turn coordination.
- Network egress, webhook listener setup, or secret storage.

## Validation

- Fast local check: `cargo test -p ironclaw_telegram_v2_adapter`
- Run `cargo test -p ironclaw_product` when shared DTO assumptions change.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Keep Telegram-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve adapter outputs as untrusted parsed DTOs until host/workflow stamps trusted context.
- Add tests before widening supported Telegram payload forms.
