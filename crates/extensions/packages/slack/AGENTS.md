# Agent Map — ironclaw_slack_extension

## Start Here

- No crate-local CLAUDE.md exists yet; use this map plus `Cargo.toml` and source files.
- Read `src/lib.rs` first, then:
  - `channel.rs` — the `SlackChannelAdapter` (`ChannelAdapter`) implementation.
  - `payload.rs` — Slack Events API payload parsing/DTO handling.
  - `mrkdwn.rs` — Slack outbound mrkdwn rendering and message chunking.
  - `delivery.rs`, `attachment_transfer.rs`, `preference_targets.rs` — delivery DTOs, attachment transfer, reply-target codec.
  - Re-derive this list with `ls crates/extensions/packages/slack/src/`.
- Read upstream contracts before changing adapter behavior:
  - `crates/ironclaw_product/AGENTS.md`

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
- Run `cargo test -p ironclaw_product` when shared DTO assumptions change.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`

## Agent Notes

- Keep Slack-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve adapter outputs as untrusted parsed DTOs until host/workflow stamps trusted context.
- Approval/auth conversational handling is deferred to the owning Reborn service seam (#3094).
