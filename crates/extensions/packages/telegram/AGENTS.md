# Agent Map — ironclaw_telegram_extension (`crates/extensions/packages/telegram/`)

## Start Here

- No crate-local CLAUDE.md exists yet; use this map plus `Cargo.toml` and source files.
- This directory is the whole Telegram **package**: the crate *and* its
  `manifest.toml` live together, per PROPOSAL §5's package rule.
- Read `src/lib.rs` first, then:
  - `payload.rs` — Telegram Bot API payload normalization/DTO handling.
  - `render.rs` — Telegram outbound request rendering.
  - `channel.rs` — the `TelegramChannelAdapter` (`ChannelAdapter`) implementation.
  - `attachment_transfer.rs`, `preference_targets.rs` — attachment transfer and reply-target codec.
  - Re-derive this list with `ls crates/extensions/packages/telegram/src/`.
- Read the contract before changing adapter behavior:
  - `crates/ironclaw_extension_contracts/` — `ChannelAdapter` and the surface vocabulary.

## What This Crate Owns

- The Telegram Bot API **protocol engine**: pure payload normalization and
  outbound request rendering, with no I/O and no secrets. Merged in from the
  former `ironclaw_telegram_v2_adapter` crate by Wave 2's package colocation
  (CHECKLIST WS2), which gave Telegram the one-crate-per-package shape Slack
  already had.
- The Telegram `ChannelAdapter` implementation: live inbound/outbound plus the
  webhook registration hooks. It is a plain native crate (no WASM target), and
  the contract is `ironclaw_extension_contracts::ChannelAdapter` — there is no
  `ProductAdapter` trait in this codebase.
- Adapter-specific mapping between Telegram shapes and the shared channel DTOs.
- Staying free of raw token bytes: hosts run the manifest-declared
  `shared_secret_header` verification and inject credentials on mediated egress.

## Dependency Rule

The package's `ironclaw_*` dependency set is exactly Slack's — `host_api`,
`extension_contracts`, `product_contracts`, `attachments` — and nothing else.
No `ironclaw_assistant`, no registry, no extension host: a concrete package crate
is linked only by the binary and by tests
(`concrete_extension_crates_link_only_from_the_binary_and_tests`).

## Do Not Move In Here

- Shared channel contracts, registry semantics, or product workflow orchestration.
- Host auth minting, canonical conversation/thread binding, or turn coordination.
- Network egress, webhook listener setup, or secret storage.

## Validation

- Fast local check: `cargo test -p ironclaw_telegram_extension`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`

## Agent Notes

- Keep Telegram-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve adapter outputs as untrusted parsed DTOs until host/workflow stamps trusted context.
- Add tests before widening supported Telegram payload forms.
