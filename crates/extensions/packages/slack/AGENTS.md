# Agent Map — ironclaw_slack_extension (`crates/extensions/packages/slack/`)

## Start Here

- Read `README.md` for orientation (surfaces, vendor, runtime, tests); this
  map plus `Cargo.toml` and source files carry the working rules.
- This directory is the whole Slack **package**: the adapter crate, its
  `manifest.toml`, `prompts/`, and the WASM user-token tools (`wasm/` +
  `wasm-src/`) live together, per the family's self-containment rule.
- Read `src/lib.rs` first, then:
  - `channel.rs` — the `SlackChannelAdapter` (`ChannelAdapter`) implementation.
  - `payload.rs` — Slack Events API payload parsing/DTO handling.
  - `mrkdwn.rs` — Slack outbound mrkdwn rendering and message chunking.
  - `delivery.rs`, `attachment_transfer.rs`, `preference_targets.rs` — delivery DTOs, attachment transfer, reply-target codec.
  - Re-derive this list with `ls crates/extensions/packages/slack/src/`.
- Read the contract before changing adapter behavior:
  - `crates/contracts/ironclaw_extension_contracts/` — `ChannelAdapter` and the
    surface vocabulary (a channel package depends on contracts-tier crates
    only; never on `ironclaw_assistant`, the registry, or the extension host).

## What This Crate Owns

- The Slack `ChannelAdapter` implementation for Reborn (issue #3857). The
  contract is `ironclaw_extension_contracts::ChannelAdapter` — there is
  no `ProductAdapter` trait in this codebase.
- Slack Events API payload parsing and outbound `chat.postMessage` rendering.
- Adapter-specific mapping between Slack shapes and the shared channel DTOs.

## Do Not Move In Here

- Legacy v1 `Channel` lifecycle, channel relay state, or host-side Slack setup/OAuth flows.
- Host auth verification, canonical conversation/thread binding, or turn coordination.
- Network clients, raw Slack bot tokens/signing secrets, direct DB/filesystem access, or approval-run state.

## Validation

- Fast local check: `cargo test -p ironclaw_slack_extension`
- Run `cargo test -p ironclaw_assistant` when shared DTO assumptions change.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`

## Agent Notes

- Keep Slack-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve adapter outputs as untrusted parsed DTOs until host/workflow stamps trusted context.
- Approval/auth conversational handling is deferred to the owning Reborn service seam (#3094).
