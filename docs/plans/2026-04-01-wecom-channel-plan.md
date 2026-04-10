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

## Bot-Primary Follow-up Direction

Now that the callback + Agent API path exists, the next phase should move
IronClaw toward the same model used by OpenClaw and the community plugin:

- WeCom Bot WebSocket as the primary inbound and in-conversation reply path
- WeCom Agent API as the proactive send and media fallback path
- self-built app callback as an optional parallel inbound path, not the only
  primary transport

The critical design goal for this phase is to avoid introducing WeCom-specific
host special cases. The host should support multiple websocket protocol modes,
while the `wecom` channel remains responsible for WeCom message parsing and
reply construction.

### Current Bot-First Status

The repository now has the first slice of this Bot-first phase in place:

- the WASM host websocket runtime supports protocol modes for both
  `discord-gateway` and `wecom-aibot`
- the WIT host interface exposes a generic `websocket-send-text` function so a
  channel can reply over an active websocket session without a WeCom-specific
  host API
- `wecom.capabilities.json` is now Bot-first:
  - `wecom_bot_id` and `wecom_bot_secret` are required
  - self-built app and callback secrets remain optional fallback transports
- the `wecom` channel now consumes websocket callback frames in `on-poll`
  and can send text replies over the Bot websocket path in `on-respond`
- websocket inbound coverage now includes core Bot message shapes beyond plain
  text, including `image`, `voice`, `file`, `video`, `mixed`, and quoted-text
  context passthrough
- websocket event handling now includes welcome-entry plus basic interactive
  event mapping for template-card and feedback callbacks

Still pending after this slice:

- richer Bot reply coverage such as stream/markdown/card/media-specific reply
  helpers
- stronger req_id / ack tracking and any needed serialized reply queueing per
  conversation
- more complete event coverage and end-to-end validation against real WeCom Bot
  traffic

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

### Host WebSocket Runtime Generalization

The current websocket runtime in `src/channels/wasm/wrapper.rs` should be
treated as a reusable websocket shell plus protocol-specific adapters.

Recommended split:

- generic websocket lifecycle in host runtime:
  - connect / disconnect / reconnect
  - frame read/write loop
  - bounded raw-frame queueing into workspace
  - triggering `on-poll`
  - generic channel-to-host websocket text send
- protocol-specific state machine:
  - `discord-gateway`
  - `wecom-aibot`

Avoid:

- `if channel_name == "wecom"` branches in the host runtime
- reusing Discord `identify` / `resume` / `READY` assumptions for WeCom
- adding WeCom-only host imports when a generic websocket send primitive works

### WeCom AIBot WebSocket Protocol Notes

Based on the official `@wecom/aibot-node-sdk`, the Bot WebSocket path looks like:

- default websocket URL:
  - `wss://openws.work.weixin.qq.com`
- auth frame:
  - `{"cmd":"aibot_subscribe","headers":{"req_id":"..."},"body":{"bot_id":"...","secret":"..."}}`
- heartbeat frame:
  - `{"cmd":"ping","headers":{"req_id":"..."}}`
- inbound message callback:
  - `cmd = "aibot_msg_callback"`
- inbound event callback:
  - `cmd = "aibot_event_callback"`
- auth / heartbeat ack:
  - no `cmd`, but `headers.req_id` plus `errcode` / `errmsg`

This differs enough from Discord that it should be implemented as a first-class
protocol mode rather than as more logic inside the existing Discord state
machine.

### Host ABI Addition

To support Bot-first replies cleanly, the WASM channel host should expose a
generic websocket send primitive via WIT, for example:

- `websocket-send-text(payload: string) -> result<_, string>`

This allows `on-respond` to send a Bot reply frame through the active websocket
connection without introducing a WeCom-only host API.

### WeCom Channel Responsibilities In Bot Phase

The `wecom` channel should own:

- parsing `aibot_msg_callback` and `aibot_event_callback` frames in `on-poll`
- mapping Bot message/event payloads to `IncomingMessage`
- building WeCom Bot reply frames in `on-respond`
- deciding when to use Bot reply vs Agent API fallback

The host should not interpret WeCom business payloads beyond websocket protocol
control behavior.

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
- `bot_transport_enabled`
- `callback_transport_enabled`

### Secret Model

Secrets should be split by transport, with Bot credentials treated as the
primary path in the Bot-first phase:

- Bot WS:
  - `wecom_bot_id`
  - `wecom_bot_secret`

- Agent API:
  - `wecom_corp_id`
  - `wecom_corp_secret`
  - `wecom_agent_id`

- Callback inbound:
  - `wecom_callback_token`
  - `wecom_callback_encoding_aes_key`

Keep secrets owner-scoped like other WASM channels.

Practical setup posture:

- Bot-only should be enough for a minimal chat bot experience
- Agent credentials should remain optional but strongly recommended for media and
  proactive sends
- callback credentials should be optional when users want parallel self-built app
  inbound

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

Recommended Phase 2 implementation order:

1. generalize host websocket runtime into protocol modes
2. add generic websocket send host capability
3. add `wecom-aibot` websocket capability config
4. implement WeCom Bot inbound parsing in `on-poll`
5. implement WeCom Bot text reply path in `on-respond`
6. preserve Agent API as fallback / supplement
7. keep callback inbound optional and parallel

This keeps us aligned with the external reference direction without blocking the
first PR on host websocket runtime changes.

## Capability File Shape

`wecom.capabilities.json` should likely include:

- channel metadata
- auth display info
- setup secrets
- HTTP allowlist for WeCom APIs
- websocket config with explicit protocol mode
- owner-scoped config defaults

For the Bot-first phase, prefer an explicit websocket config shape such as:

- `protocol = "wecom-aibot"`
- `url = "wss://openws.work.weixin.qq.com"`
- `connect_on_start = true`
- `bot_id_secret_name = "wecom_bot_id"`
- `bot_secret_name = "wecom_bot_secret"`
- optional heartbeat / reconnect tuning if the runtime exposes it

Potential domains will depend on the final WeCom endpoints, but keep them tightly allowlisted.

## Suggested Development Order

1. Generalize host websocket runtime into protocol modes
2. Add generic websocket text send host capability
3. Extend `wecom.capabilities.json` for Bot WS primary config
4. Implement WeCom Bot WS connect/auth/heartbeat lifecycle
5. Implement Bot text inbound -> emit message
6. Implement Bot text outbound reply
7. Restore Agent API as fallback / supplement
8. Add image/file/voice/video Bot inbound coverage
9. Add req_id + msg_id dedupe
10. Revisit merge window after runtime timing support
11. Add docs and parity updates

## Tests

Initial test targets:

- websocket runtime protocol parsing for `discord-gateway` vs `wecom-aibot`
- WeCom auth / heartbeat / ack frame handling
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

- exact Agent API media upload flow
- whether Bot WS supports all required reply media types
- whether callback inbound should stay enabled by default once Bot WS exists, or
  be opt-in parallel inbound
- whether quoted-message context should be part of MVP
