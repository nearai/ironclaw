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

## Previous IronClaw Constraint

This branch started from a real host limitation: IronClaw's WASM websocket
runtime was originally shaped around the Discord gateway lifecycle:

- fixed identify payload wrapping
- built-in heartbeat / resume expectations
- queue processing tuned for Discord-style frames

That made callback + Agent API the practical first slice before WeCom Bot WS.

That constraint has now been partially lifted on this branch:

- the host websocket runtime has been generalized into protocol modes
- `wecom-aibot` now exists alongside `discord-gateway`
- a generic WIT websocket send primitive is available for channel replies

The original callback + Agent API MVP still matters, but it is no longer the
only viable transport shape on this branch.

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
- Bot passive replies now use the WeCom SDK-aligned `stream` reply shape
- websocket inbound coverage now includes core Bot message shapes beyond plain
  text, including `image`, `voice`, `file`, `video`, `mixed`, and quoted-text
  context passthrough
- websocket event handling now includes welcome-entry plus basic interactive
  event mapping for template-card and feedback callbacks
- the self-built app / Agent API path is still active and currently serves as:
  - proactive send path
  - attachment/media fallback path
  - optional callback inbound path
- local real-world validation has already covered:
  - gateway web chat request flow
  - real WeCom Bot single-DM inbound
  - real WeCom Bot passive text reply

Still pending after this slice:

- richer Bot reply coverage such as markdown/card/media-specific reply helpers
- stronger req_id / ack tracking and any needed serialized reply queueing per
  conversation
- explicit failure UX for provider / turn errors so the user gets a visible
  reply instead of a silent or long-lived processing state
- more complete event coverage and end-to-end validation against real WeCom Bot
  traffic, especially group conversations
- richer voice handling when WeCom does not already provide recognized text

### Current Behavioral Boundaries

Important real boundaries on the current implementation:

- single-DM Bot text chat is the primary supported path right now
- group chat text should work over Bot WS, but group attachment reply behavior
  is not complete yet because the current attachment fallback path still depends
  on Agent API direct-recipient routing
- group-chat safety hardening is not complete yet:
  - current sender admission is evaluated per sender identity
  - but WeCom group chats do not yet have a dedicated DM-only guard for
    approval-required tools
  - approval prompts rendered as ordinary chat replies remain a poor fit for
    shared chats
- callback voice handling currently stores AMR media and uses WeCom's
  `Recognition` field when present
- Bot websocket voice handling currently consumes the provided text content
  field; this branch does not yet introduce a raw audio transcoding pipeline for
  AMR / SILK voice payloads
- Agent API remains required for:
  - proactive sends
  - attachment sends
  - callback-only deployments

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

### Phase 1 Completed

Use callback + Agent API for:

- receive messages
- send replies
- media download and upload

This slice is already implemented on this branch.

### Phase 2 In Progress

Add Bot WS for:

- bot-session primary inbound
- richer reply behavior
- closer parity with OpenClaw's official path

Current completed pieces of Phase 2:

- generalized websocket host runtime
- generic websocket send host capability
- `wecom-aibot` websocket capability config
- WeCom Bot inbound parsing in `on-poll`
- WeCom Bot text reply path in `on-respond`
- Agent API preserved as fallback / supplement
- callback inbound preserved as optional parallel path

Recommended Phase 2 implementation order:

1. add explicit WeCom group-chat safety hardening:
   - evaluate admission in shared chats per sender identity
   - do not emit pairing codes in group chats
   - reject approval-required tools in group chats and direct users to DM
   - avoid rendering approval prompts back into shared chats
2. harden req_id / ack handling and any serialized reply queueing needed for Bot conversations
3. expand richer Bot reply shapes beyond plain stream text
4. close the group-chat attachment / fallback gap
5. improve provider-failure reply behavior and turn-finalization UX
6. extend event coverage and real-traffic validation
7. revisit raw voice / transcription pipeline only if WeCom-provided text is not sufficient

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

## Suggested Remaining Development Order

1. Add WeCom group-chat safety hardening:
   - sender allowlist checks in group chats
   - no pairing-code replies in shared chats
   - no approval-required tools in shared chats
2. Harden req_id / ack tracking for Bot replies
3. Add richer Bot outbound reply shapes (`markdown`, card, richer media reply strategy)
4. Close group-chat attachment reply limitations
5. Improve failure handling so provider / turn errors surface as explicit user-visible replies
6. Expand real WeCom group / media / event fixtures and end-to-end validation
7. Revisit merge window after runtime timing support
8. Evaluate whether raw AMR / SILK voice transcription is needed beyond WeCom-provided recognized text
9. Keep docs and parity notes current as each slice lands

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

Current verified coverage on this branch includes:

- protocol parsing for `wecom-aibot`
- host websocket sender injection into `on-respond`
- WeCom Bot passive reply payload construction
- callback crypto / dedupe / event-ignore behavior
- outbound media classification
- local live gateway chat validation
- real WeCom Bot single-DM inbound + passive text reply validation

Still worth adding:

- assertions that WeCom group chats deny approval-required tools and avoid
  group-visible pairing / approval prompts
- real group-chat validation
- richer media / reply-shape validation
- explicit failure-path assertions for provider errors and stuck turns

## Docs / Tracking

When implementation starts, update:

- `FEATURE_PARITY.md`
- relevant docs under `docs/plans/`
- registry entries for the new channel

## Open Questions

- whether Bot WS supports every reply/media shape we want, or whether some
  classes should remain App-only on purpose
- whether callback inbound should stay enabled by default once Bot WS exists, or
  be opt-in parallel inbound
- whether raw AMR / SILK voice should be transcoded and transcribed in-host when
  WeCom does not provide recognized text
- whether external-channel threads should remain visible-but-readonly in the web
  gateway, or be hidden / specially presented in the UI

## Group Chat Security Direction

WeCom group chats should follow a stricter posture than single-DM Bot chats.

Recommended policy:

- evaluate admission in shared chats per sender identity, not by group alone
- require explicit admission before a sender can drive bot behavior in a shared
  chat
- do not emit pairing codes into a group chat; if a sender is not admitted,
  drop the message or return a minimal DM-me-first response
- deny approval-required or otherwise sensitive tools in group chats, even when
  ordinary chat is allowed
- direct users to DM the bot for sensitive actions instead of trying to
  complete approval flows in a shared thread

Existing repo patterns suggest a combined model:

- Telegram is the reference for shared-chat admission:
  - group chats still evaluate sender allowlists
  - pairing replies are only emitted in private chats
- relay/shared channels are the reference for approval-sensitive tools:
  - approval-required tools are rejected in non-DM shared contexts

WeCom should follow this Telegram + relay posture rather than the looser
Discord DM-pairing-only behavior.
