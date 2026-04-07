# WeCom Channel Plan

## Goal

Add a new standalone `wecom` WASM channel for Enterprise WeChat / WeCom.

This should be a separate channel from `wechat`.

Reasons:
- different protocol surface
- different auth/config model
- different lifecycle and delivery modes
- avoids `if wechat else if wecom` branching inside one channel

## External Reference Direction

Reference implementations and docs indicate a split model:

- WeCom Bot WebSocket as the primary inbound / in-conversation reply path
- WeCom Agent API as the proactive send / media fallback path

Useful references:
- OpenClaw WeCom guide
- Tencent Cloud OpenClaw + WeCom integration guide
- `sunnoy/openclaw-plugin-wecom` community plugin

## Current IronClaw Constraint

IronClaw's existing WASM websocket runtime is currently shaped around the
Discord gateway lifecycle:

- fixed identify payload wrapping
- built-in heartbeat / resume expectations
- queue processing tuned for Discord-style frames

That means WeCom Bot WebSocket should be treated as a follow-up transport task,
not the first implementation slice, unless the host websocket runtime is
generalized first.

Because of this, the practical MVP path in IronClaw is:

- inbound via WeCom self-built app HTTP callback
- outbound via WeCom Agent API

This still gives us a useful enterprise WeChat channel while preserving a clean
upgrade path to Bot WS later.

## Recommended Scope

### MVP

Build a new `channels-src/wecom` channel with:

- self-built app HTTP callback inbound
- Agent API outbound
- text inbound
- text outbound
- image inbound
- file inbound
- voice inbound
- message deduplication
- short attachment merge window

Use Bot WebSocket later in MVP+1 for:

- primary bot-session inbound
- in-conversation bot reply path
- richer bot-specific event handling

### Explicitly Out of Scope for Initial PR

- multi-account
- org directory sync
- bot websocket inbound / bot-session transport
- advanced sender routing
- video understanding
- broad enterprise admin UX

## Architecture

### New Channel

Create:

- `channels-src/wecom/Cargo.toml`
- `channels-src/wecom/build.sh`
- `channels-src/wecom/wecom.capabilities.json`
- `channels-src/wecom/src/lib.rs`

Likely supporting modules:

- `channels-src/wecom/src/types.rs`
- `channels-src/wecom/src/api.rs`
- `channels-src/wecom/src/ws.rs`
- `channels-src/wecom/src/state.rs`
- `channels-src/wecom/src/media.rs`

### Why Separate From Existing Channels

- current repo has no personal WeChat channel implementation on this branch
- Feishu and Telegram already show the intended WASM channel shape
- WeCom is closer to a fresh external channel than a variant of Feishu or Telegram

## Config Model

Suggested config shape:

- `bot_id`
- `bot_secret`
- `corp_id`
- `corp_secret`
- `agent_id`
- `owner_id`
- `dm_policy`
- `allow_from`
- `polling_enabled` or equivalent reconnect control
- `inbound_merge_window_ms`

### Secret Model

Required secrets for MVP should be split by transport:

- Bot WS:
  - `wecom_bot_id`
  - `wecom_bot_secret`

- Agent API:
  - `wecom_corp_id`
  - `wecom_corp_secret`
  - `wecom_agent_id`

Keep secrets owner-scoped like other WASM channels.

## Runtime State

Persist owner-scoped channel state under a workspace prefix like:

- `channels/wecom/`

Likely state items:

- bot websocket session metadata
- token cache
- token expiry
- dedupe state (`msg_id`, `req_id`)
- pending inbound bundle state

## Message Model

### Inbound MVP Types

- text
- image
- file
- voice

### Deferred

- video as a first-class media path
- richer group mention semantics
- quoted-message context

### Attachment Handling

Follow the current repo's generic attachment model:

- image -> image attachment
- file -> document/file attachment
- voice -> audio attachment
- video -> ordinary file attachment for now if needed later

### Merge Window

Use a short merge window for:

- attachment followed by text

Suggested initial default:

- `3000-5000ms`

This should be configurable in `wecom.capabilities.json`.

Current constraint on this branch:

- IronClaw's WASM channel polling interval has a minimum of 30000ms
- that makes a 3000-5000ms callback merge window impractical without new
  host-side timer or flush support
- defer merge-window behavior until the host runtime can support sub-30s
  callback bundle flushing cleanly

## Delivery Strategy

### Phase 1

Use callback + Agent API for:

- receive messages
- send replies
- media download and upload

### Phase 2

Add Bot WS for:

- bot-session primary inbound
- richer reply behavior
- closer parity with OpenClaw's official path

This keeps us aligned with the external reference direction without blocking the
first PR on host websocket runtime changes.

## Capability File Shape

`wecom.capabilities.json` should likely include:

- channel metadata
- auth display info
- setup secrets
- HTTP allowlist for WeCom APIs
- optional websocket-related config if needed by runtime
- owner-scoped config defaults

Potential domains will depend on the final WeCom endpoints, but keep them tightly allowlisted.

## Suggested Development Order

1. Scaffold `channels-src/wecom`
2. Add `wecom.capabilities.json`
3. Register in registry / bundles
4. Implement config parsing and workspace state
5. Implement Bot WS connect/auth lifecycle
6. Implement text inbound -> emit message
7. Implement text outbound reply
8. Add image/file/voice inbound
9. Add dedupe
10. Add merge window
11. Add Agent API outbound supplement
12. Add docs and parity updates

## Tests

Initial test targets:

- config parsing
- token expiry / refresh behavior
- message dedupe state
- attachment bundle merge state machine
- mapping WeCom inbound payloads to `IncomingMessage`
- outbound request payload construction

## Docs / Tracking

When implementation starts, update:

- `FEATURE_PARITY.md`
- relevant docs under `docs/plans/`
- registry entries for the new channel

## Open Questions

- exact WeCom Bot WS auth handshake shape
- exact Agent API media upload flow
- whether Bot WS supports all required reply media types
- whether callback inbound should be added in PR 1 or deferred
- whether quoted-message context should be part of MVP
