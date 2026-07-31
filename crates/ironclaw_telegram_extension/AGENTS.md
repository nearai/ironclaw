# Agent Map — ironclaw_telegram_extension

## Start Here

- No crate-local CLAUDE.md exists yet; use this map plus `Cargo.toml` and source files.
- Read `src/lib.rs` first, then:
  - `channel.rs` — the `TelegramChannelAdapter` (`ChannelAdapter`) implementation.
  - `attachment_transfer.rs`, `preference_targets.rs` — attachment transfer and reply-target codec.
  - Payload parsing and outbound rendering are **not** in this crate; they live
    in the sibling `ironclaw_telegram_v2_adapter`.
  - Re-derive this list with `ls crates/ironclaw_telegram_extension/src/`.
- Read upstream contracts before changing adapter behavior:
  - `crates/ironclaw_product/AGENTS.md`

## What This Crate Owns

- The Telegram `ChannelAdapter` implementation: live inbound/outbound plus the
  webhook registration hooks. It is a plain native crate (no WASM target), and
  the contract is `ironclaw_host_api::product_adapter::ChannelAdapter` — there
  is no `ProductAdapter` trait in this codebase.
- Adapter-specific mapping between Telegram shapes and the shared channel DTOs.
- Staying free of raw token bytes: hosts run the manifest-declared
  `shared_secret_header` verification and inject credentials on mediated egress.

## Do Not Move In Here

- Shared ProductAdapter contracts, registry semantics, or product workflow orchestration.
- Host auth minting, canonical conversation/thread binding, or turn coordination.
- Network egress, webhook listener setup, or secret storage.

## Validation

- Fast local check: `cargo test -p ironclaw_telegram_extension`
- Run `cargo test -p ironclaw_product` when shared DTO assumptions change.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Keep Telegram-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve adapter outputs as untrusted parsed DTOs until host/workflow stamps trusted context.
- Add tests before widening supported Telegram payload forms.
