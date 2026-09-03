# Agent Map — ironclaw_slack_extension (`crates/extensions/packages/slack/`)

## Start Here

- Read `README.md` for orientation (surfaces, vendor, runtime, tests); this
  map plus `Cargo.toml` and source files carry the working rules.
- This directory is the whole Slack **package**: the channel-capability crate, its
  `manifest.toml`, `prompts/`, and the WASM user-token tools (`wasm/` +
  `wasm-src/`) live together, per the family's self-containment rule.
- Read `src/lib.rs` first, then:
  - `channel.rs` — the `SlackChannelAdapter` implementations of
    `ChannelIngress` and `ChannelDelivery`.
  - `reply_sink/` — the `ReplySink` half: the run's answer on Slack's
    native Agent surface (`agents.sessions.setStatus`, `chat.startStream` /
    `appendStream` / `stopStream`, task cards, read-back after ambiguity).
    `mod.rs` is the reconciler; `plan.rs` the chunk planner; `checkpoint.rs`
    the checkpoint version and shape; `agent_api.rs` the request/outcome
    mapping.
  - `reply_context.rs` — the reply context ingress stamps on every message
    (recipient ids + session thread) and the sink reads at reply time.
  - `api.rs` — the one inventory of Slack Web API endpoints the package
    calls; `tests/agent_app_manifest_lockstep.rs` pins it against the
    manifest's `[[channel.egress]]` and the documented app manifest.
  - `payload.rs` — Slack Events API payload parsing/DTO handling, including
    the Agent event family (`agent_session_stopped` → the declared `stop`
    command; the rest are authenticated no-ops with their own reason).
  - `mrkdwn.rs` — Slack outbound mrkdwn rendering and message chunking.
  - `delivery.rs`, `attachment_transfer.rs`, `preference_targets.rs` — delivery DTOs, attachment transfer, reply-target codec.
  - Re-derive this list with `ls crates/extensions/packages/slack/src/`.
- Read the contract before changing channel behavior:
  - `crates/contracts/ironclaw_extension_contracts/` — `ChannelIngress`,
    `ChannelDelivery`, `reply::ReplySink`, and the
    surface vocabulary (a channel package depends on contracts-tier crates
    only; never on `ironclaw_assistant`, the registry, or the extension host).
  - `docs/internal/design/2026-08-31-progressive-reply-publication.md` §6
    (the sink seam) and §9 (the Slack native Agent plan this package
    implements). Verified Slack facts are quoted at the top of
    `src/reply_sink/mod.rs`; re-fetch the docs.slack.dev pages before changing
    a request shape.

## What This Crate Owns

- Slack's three channel capability implementations for Reborn (issue #3857).
  `receive` completes payload-derived attachments and shared-conversation
  context through restricted egress before returning; `deliver` renders and
  sends message-shaped notices; the reply sink reconciles Slack's native
  Agent surface toward each reply revision behind its own checkpoint. There
  is no `ProductAdapter` trait in this codebase.
- Slack Events API payload parsing, outbound `chat.postMessage` rendering,
  and the streamed reply (`chat.*Stream`, `agents.sessions.setStatus`).
- Adapter-specific mapping between Slack shapes and the shared channel DTOs.
- The no-fallback rule: a workspace whose app is not an Agent
  (`feature_disabled`/`not_agent_app`) fails the reply with a `Permanent`
  outcome naming the missing capability; never post a conventional message
  in its place.

## Do Not Move In Here

- Legacy v1 `Channel` lifecycle, channel relay state, or host-side Slack setup/OAuth flows.
- Host auth verification, canonical conversation/thread binding, or turn coordination.
- Network clients, raw Slack bot tokens/signing secrets, direct DB/filesystem access, or approval-run state.

## Validation

- Fast local check: `cargo test -p ironclaw_slack_extension` — includes the
  conformance suite (`tests/channel_conformance.rs`, stream cadence), the
  reply sink against the in-crate fake Agent API
  (`tests/reply_sink_agent_api.rs`, exact request bodies), and the
  manifest/docs lockstep (`tests/agent_app_manifest_lockstep.rs`).
- Run `cargo test -p ironclaw_assistant` when shared DTO assumptions change.
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture_tests`

## Agent Notes

- Keep Slack-specific parsing/rendering here; move reusable DTO concerns upstream.
- Preserve package outputs as untrusted complete DTOs until host/workflow validates and stamps trusted context.
- Approval/auth conversational handling is deferred to the owning Reborn service seam (#3094).
