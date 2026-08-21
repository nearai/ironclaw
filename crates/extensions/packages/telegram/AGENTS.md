# Agent Map — ironclaw_telegram_extension (`crates/extensions/packages/telegram/`)

## Start Here

- Read `README.md` for orientation (surfaces, vendor, runtime, tests); this
  map plus `Cargo.toml` and source files carry the working rules.
- This directory is the whole Telegram **package**: the crate *and* its
  `manifest.toml` live together, per PROPOSAL §5's package rule.
- Read `src/lib.rs` first, then:
  - `payload.rs` — Telegram Bot API payload normalization/DTO handling.
  - `render.rs` — Telegram outbound request rendering.
  - `channel.rs` — the `TelegramChannelAdapter` implementations of
    `ChannelIngress`, `ChannelReply`, and `ChannelDelivery`.
  - `attachment_transfer.rs`, `preference_targets.rs` — attachment transfer and reply-target codec.
  - `linked/` — MTProto device login, session handling, Telegram mappings,
    connection pooling, and the 15 linked-account tool operations.
  - Re-derive this list with `ls crates/extensions/packages/telegram/src/`.
- Read the contract before changing channel behavior:
  - `crates/contracts/ironclaw_extension_contracts/` — `ChannelIngress`,
    `ChannelReply`, `ChannelDelivery`, and the surface vocabulary.

## What This Crate Owns

- The Telegram Bot API **protocol engine**: pure payload normalization and
  outbound request rendering, with no I/O and no secrets. Merged in from the
  former `ironclaw_telegram_v2_adapter` crate by Wave 2's package colocation
  (CHECKLIST WS2), which gave Telegram the one-crate-per-package shape Slack
  already had.
- Telegram's three channel capability implementations. `receive` completes the
  two-hop Bot API file exchange through restricted egress; reply/delivery
  render and send. Webhook registration/deregistration are manifest recipes
  executed by the generic host. This is a plain native crate (no WASM target),
  and there is no `ProductAdapter` trait in this codebase.
- Adapter-specific mapping between Telegram shapes and the shared channel DTOs.
- Staying free of raw token bytes: hosts run the manifest-declared
  `shared_secret_header` verification and inject credentials on mediated egress.
- The Telegram-specific half of linked-account access: phone/QR device login,
  session serialization, validated MTProto transport, live reads, and personal-
  account writes for the manifest's 15 tools. Generic device-link orchestration,
  durable linked-session custody, authorization, and capability mediation stay
  host-owned.

## Dependency Rule

The package's `ironclaw_*` dependency set is exactly Slack's — `host_api`,
`extension_contracts`, `product_contracts`, `attachments` — and nothing else.
No `ironclaw_assistant`, no registry, no extension host: a concrete package crate
is linked only by the binary and by tests
(`concrete_extension_crates_link_only_from_the_binary_and_tests`).

### The `grammers-*` edges are a security pin, not a version range

The linked-device (MTProto) half links `grammers-*`, which runs **in-process
with full process authority** — it can read the heap that holds every user's
decrypted session key. Three rules, and none of them is style:

- Every edge is `=0.10.0` exactly, with `default-features = false` and an
  explicit feature allowlist. `0.10.0` is the last release whose `update_config`
  discards Telegram's server-pushed datacenter list; that is the only reason
  `Session::dc_option` gates 100% of dials.
- The socks5 `proxy` feature must stay **off**. A proxied dial never reaches
  that seam, so enabling it falsifies the validation claim rather than weakening
  it.
- Versions, `.crate` checksums, and the *resolved* feature sets (including the
  five transitive members no manifest can pin) are frozen by
  `crates/app/ironclaw_architecture_tests/tests/reborn_linked_device_supply_chain_pin.rs`,
  and `.github/dependabot.yml` ignores `grammers-*` so no bot proposes a bump.
  Read that file's module docs — and the bump checklist in it — before changing
  any of this. Design record:
  `docs/internal/design/telegram-linked-device/{PROPOSAL.md §11.1, ADR-device-link-auth-hook.md}`.

## Do Not Move In Here

- Shared channel contracts, registry semantics, or product workflow orchestration.
- Host auth minting, canonical conversation/thread binding, or turn coordination.
- Network egress, webhook listener setup, or secret storage.

## Validation

- Fast local check: `cargo test -p ironclaw_telegram_extension`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`

## Agent Notes

- Keep Telegram-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve package outputs as untrusted complete DTOs until host/workflow validates and stamps trusted context.
- Add tests before widening supported Telegram payload forms.
