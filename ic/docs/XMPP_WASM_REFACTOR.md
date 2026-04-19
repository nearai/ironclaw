# XMPP WASM Refactor Draft

Status: phase 1 implemented in-tree

## Current Status

The first implementation slice in this document is now landed:

- `channels-src/xmpp/` exists as an installable WASM channel package
- `registry/channels/xmpp.json` exists for extension-style installation
- `bridges/xmpp-bridge/` exists as the local loopback bridge
- channel setup now supports `required_fields` in addition to `required_secrets`
- native daemon startup no longer starts `XmppChannel` directly in normal runtime
- legacy native `XMPP_*` config is migrated into installable-channel secrets and setup fields

Remaining gaps:

- the bridge is still user-managed rather than host-managed
- bridge-owned DM OMEMO now has in-tree bootstrap/decrypt/persisted-session coverage, but MUC OMEMO and broader external-client interoperability validation are still outstanding
- extension inventory and UX now use the installable path, but managed sidecar lifecycle is still future work

## Goal

Move XMPP from a native daemon-only channel into the same installable lifecycle as custom WASM channels like `weechat`:

- installable from `registry/channels/xmpp.json`
- built as a separate `xmpp.wasm` channel artifact
- configured through `setup.required_secrets` and channel setup fields
- visible to the agent through the same extension/channel inventory as other installable channels
- restart-safe with persisted state and pairing data

The target is not "port the current native module byte-for-byte into WASM". The target is "make XMPP behave like an installable channel in the product".

## Current Problem

XMPP currently lives in the native channel bucket:

- native Rust module: `src/channels/xmpp/`
- direct startup path in `src/main.rs`
- config loaded from `config.channels.xmpp`
- not represented in the installable extension registry

That breaks the product model established by custom channels like `weechat` and `darkirc`, where the channel is:

- separately compiled
- installed into `~/.ironclaw/channels/`
- configured by capabilities-driven setup
- surfaced through extension/channel management

## Hard Constraint

The current WASM channel runtime is designed around:

- HTTP webhooks
- periodic polling
- message emission
- scoped workspace state
- host-managed secret injection

It is not a good fit for a raw long-lived XMPP client session with:

- direct TCP/TLS sockets
- XML stream negotiation
- presence and MUC state
- stream reconnection
- OMEMO session/key management

Because of that, a pure "run the XMPP client directly inside WASM" design is the wrong refactor target.

## Decision

Adopt a two-part XMPP channel architecture:

1. `xmpp.wasm` becomes the installable IronClaw channel.
2. A local XMPP bridge process owns the actual XMPP protocol session.

This matches the pattern used by `weechat`: the installable channel talks to an external relay/bridge over supported host capabilities instead of owning the network protocol stack directly.

## Target Architecture

```text
IronClaw agent
    ^
    |
ChannelManager
    ^
    |
xmpp.wasm
    |  HTTP polling + send API + optional webhook
    v
local xmpp-bridge
    |  TCP/TLS + XMPP stream + MUC + OMEMO
    v
XMPP server
```

### Responsibilities

`xmpp.wasm` owns:

- setup schema and installable channel packaging
- DM/group policy enforcement
- pairing flow integration
- normalization into `IncomingMessage`
- metadata contract for replies
- persistence of lightweight channel state such as watermarks and bridge cursors

`xmpp-bridge` owns:

- XMPP login and reconnect loop
- stanza parsing and serialization
- MUC joins and participant tracking
- outbound send queue
- OMEMO device/session/key management
- server capability discovery

## Why A Bridge Is Required

This is the concrete reason XMPP differs from `telegram`, `slack`, or `weechat` today.

The current WASM runtime already supports:

- `on_start`
- `on_poll`
- `on_http_request`
- `on_respond`
- host HTTP requests with secret placeholders

That makes it a good fit for:

- webhook APIs
- poll-based APIs
- localhost relays

It does not currently provide:

- raw sockets
- TLS streams
- XMPP stream management
- long-lived async task ownership outside the polling model

So the refactor should move XMPP toward the installable model by splitting protocol ownership out of the daemon, not by forcing the raw protocol into the current WASM surface.

## Packaging Model

Add a new installable channel:

- `registry/channels/xmpp.json`
- `channels-src/xmpp/`
- `channels-src/xmpp/xmpp.capabilities.json`
- `channels-src/xmpp/build.sh`
- `channels-src/xmpp/src/lib.rs`

Recommended sidecar source location:

- `bridges/xmpp-bridge/`

or, if you want it versioned like your custom channels:

- move the bridge into its own repo and let the channel talk to it exactly like `weechat` talks to its relay.

## Proposed Install Lifecycle

### Install

The user installs `xmpp` like any other channel:

- registry entry resolves to `xmpp.wasm`
- artifact copied into `~/.ironclaw/channels/xmpp.wasm`
- capabilities copied into `~/.ironclaw/channels/xmpp.capabilities.json`

### Configure

The setup modal/onboarding flow prompts for XMPP secrets and fields.

### Activate

Activation does one of two things:

1. Phase 1, simpler:
   - assumes a user-managed bridge at `bridge_url`
   - `xmpp.wasm` verifies connectivity and starts polling
2. Phase 2, fuller product:
   - host launches a managed local `xmpp-bridge`
   - injects secrets into the bridge environment
   - `xmpp.wasm` talks to the managed bridge over loopback

Phase 1 gets XMPP into the installable model quickly. Phase 2 removes the manual bridge step.

Phase 2 requires a real host-side sidecar lifecycle, not just channel setup changes. That should be implemented as a generic extension-owned process facility rather than XMPP-only ad hoc startup logic.

## Secrets Model

Use existing encrypted secrets storage, just like other WASM channels.

### Required secrets

Initial required secrets:

- `xmpp_jid`
- `xmpp_password`

Optional generated secrets:

- `xmpp_bridge_token`
  - used for loopback bridge auth if the bridge exposes HTTP
- `xmpp_webhook_secret`
  - used only if the bridge pushes inbound messages via webhook instead of polling

### Optional future secrets

- `xmpp_client_cert`
- `xmpp_client_key`
- `xmpp_omemo_store_passphrase`

### Secret delivery

Reuse existing channel secret patterns:

1. `setup.required_secrets` prompts the user and stores encrypted values.
2. `inject_channel_credentials()` injects secrets into placeholders when the WASM channel makes HTTP requests.
3. For secrets that must become runtime config values rather than HTTP placeholders, add XMPP-specific config injection just like the existing `feishu` special case in `src/channels/wasm/setup.rs`.

## Non-Secret Setup Fields

XMPP needs non-secret config beyond `jid` and `password`. Today, WASM channels do not surface `required_fields` the way WASM tools do. That must change.

### Required host change

Extend `ChannelCapabilitiesFile.setup` to support channel `required_fields`, reusing the existing tool field schema instead of inventing another one.

Minimum field set:

- `bridge_url`
  - default: `http://127.0.0.1:7798`
- `dm_policy`
  - `allowlist | open | pairing`
- `group_policy`
  - `deny | allowlist | open`
- `allow_plaintext_fallback`
  - `true | false`
- `rooms`
  - comma-separated MUC JIDs or JSON array persisted by the UI
- `resource`
  - optional XMPP resource override

If the bridge owns direct server connection details rather than discovering from the JID domain, add:

- `xmpp_host`
- `xmpp_port`
- `xmpp_domain`

### Required code changes

- `src/channels/wasm/schema.rs`
  - add `required_fields` to channel setup schema
- `src/extensions/manager.rs`
  - surface channel setup fields in `get_setup_schema()`
  - persist channel setup fields in `configure()`
  - allow mapping selected fields into settings or extension-owned config
- `src/setup/channels.rs`
  - prompt for both secrets and fields during onboarding
- `src/setup/README.md`
  - document field support for WASM channels

## XMPP Bridge API

The bridge API should be intentionally small.

### Inbound pull API

`GET /v1/messages?cursor=<cursor>`

Returns normalized inbound messages:

```json
{
  "next_cursor": "abc123",
  "messages": [
    {
      "id": "msg-1",
      "sender": "alice@example.com",
      "target": "alice@example.com",
      "chat_type": "chat",
      "body": "hi",
      "timestamp": "2026-04-10T12:00:00Z"
    }
  ]
}
```

For MUC:

```json
{
  "id": "msg-2",
  "sender": "room@conference.example.com/alice",
  "target": "room@conference.example.com",
  "chat_type": "groupchat",
  "room": "room@conference.example.com",
  "nick": "alice",
  "body": "hello room"
}
```

### Outbound send API

`POST /v1/messages/send`

Request:

```json
{
  "target": "room@conference.example.com",
  "chat_type": "groupchat",
  "body": "hello",
  "metadata": {
    "xmpp_room": "room@conference.example.com"
  }
}
```

### Health/status API

`GET /v1/status`

Returns:

- connected/disconnected
- bound JID
- joined rooms
- encryption mode
- last error

### Optional push API

If push is desired later:

- bridge calls the channel webhook path
- webhook protected by `xmpp_webhook_secret`

Polling is sufficient for the first installable version.

## WASM Channel Behavior

### `on_start`

`xmpp.wasm` should:

- load secrets and setup fields from config
- validate bridge reachability
- return polling config
- optionally request bridge room joins

### `on_poll`

`xmpp.wasm` should:

- call `GET /v1/messages?cursor=...`
- apply DM/group policy
- apply pairing checks
- emit normalized `IncomingMessage`s
- persist `next_cursor` in workspace state

### `on_respond`

`xmpp.wasm` should:

- parse message metadata
- preserve the current XMPP routing contract:
  - `xmpp_target`
  - `xmpp_from`
  - `xmpp_type`
  - `xmpp_room`
  - `xmpp_nick`
- call bridge send API

### `on_status`

Should expose:

- bridge health
- active connection state
- room join count
- pairing mode
- plaintext/OMEMO mode

## Metadata Contract

Do not invent a new reply-routing shape. Keep the metadata contract already used by the native XMPP implementation so message routing does not regress during migration.

Required metadata keys:

- `xmpp_target`
- `xmpp_from`
- `xmpp_type`
- `xmpp_room`
- `xmpp_nick`

The WASM version should preserve these exactly so the agent and reply path continue to work the same way.

## Pairing

Pairing should stay in the WASM layer, not the bridge.

Reason:

- pairing is an IronClaw policy concept
- other installable channels already integrate pairing at the channel layer
- it keeps the bridge protocol-agnostic and easier to reuse

Flow:

1. bridge surfaces inbound DM
2. `xmpp.wasm` checks allowlist and pairing store
3. if unapproved and `dm_policy = pairing`, channel emits no user message
4. channel sends pairing instructions back through bridge send API
5. approval is persisted through the existing IronClaw pairing store

## OMEMO

Do not port the old native OMEMO code into the installable channel unchanged.

Reason:

- the long-term extension boundary must still converge on standards-compliant cross-client behavior
- OMEMO session/key handling belongs in the bridge, not the WASM layer

### Recommendation

Phase the migration:

1. Phase A:
   - installable XMPP channel with plaintext only or explicit fallback support
2. Phase B:
   - bridge-owned DM OMEMO implementation with persisted Signal sessions
3. Phase C:
   - broader interop validation, MUC OMEMO, and removal of any remaining compatibility-only assumptions

OMEMO state should be stored by the bridge under a stable per-channel data directory, not in the WASM component.

## Persistence

### WASM channel state

Persist in channel workspace:

- bridge cursor
- last seen message IDs if cursoring is not enough
- room subscriptions if not persisted in setup fields

Recommended workspace prefix:

- `channels/xmpp/`

### Bridge state

Persist separately:

- stream resumption state
- roster cache if needed
- MUC state if needed
- OMEMO keys and sessions

Recommended path:

- `~/.ironclaw/extensions/xmpp/bridge/`

or, for managed sidecars:

- `~/.ironclaw/state/channels/xmpp/`

## Migration Plan

### PR 1: Enable channel setup fields

Deliverables:

- add `required_fields` support to WASM channels
- UI support in extensions/channel setup
- onboarding support for channel fields

This is required for XMPP, and it also improves parity for other custom channels.

### PR 2: Create `xmpp.wasm`

Deliverables:

- `channels-src/xmpp/`
- `xmpp.capabilities.json`
- `registry/channels/xmpp.json`
- basic poll/send path against a local bridge
- secrets setup for `xmpp_jid` and `xmpp_password`

At this point, XMPP appears in the same extension inventory as other channels.

### PR 3: Extract bridge

Deliverables:

- standalone `xmpp-bridge`
- local HTTP API for receive/send/status
- direct XMPP login and MUC support

This can initially reuse logic extracted from the current native `src/channels/xmpp/` module.

### PR 4: Pairing parity

Deliverables:

- WASM-layer pairing flow
- metadata parity with the native implementation
- tests for DM and group routing

### PR 5: OMEMO rewrite

Deliverables:

- standards-compliant bridge-owned OMEMO
- encrypted send/receive path
- migration off the current native OMEMO implementation

## Native XMPP Decommission Plan

Once the installable channel is stable:

1. gate native XMPP behind a feature flag or hidden compatibility mode
2. stop listing native XMPP as a built-in channel
3. migrate users to the installable `xmpp` extension
4. remove `config.channels.xmpp` direct startup path

Temporary compatibility is acceptable, but two production XMPP implementations should not live indefinitely.

## Concrete File-Level Work

### New files

- `docs/XMPP_WASM_REFACTOR.md`
- `channels-src/xmpp/Cargo.toml`
- `channels-src/xmpp/build.sh`
- `channels-src/xmpp/xmpp.capabilities.json`
- `channels-src/xmpp/src/lib.rs`
- `registry/channels/xmpp.json`
- `bridges/xmpp-bridge/...`

### Existing files to change

- `src/channels/wasm/schema.rs`
- `src/channels/wasm/setup.rs`
- `src/extensions/manager.rs`
- `src/setup/channels.rs`
- `src/setup/README.md`
- `src/cli/channels.rs`
- `src/channels/web/static/app.js`
- `src/channels/web/handlers/extensions.rs`
- `src/main.rs`

### Existing native code to extract or retire

- `src/channels/xmpp/mod.rs`
- `src/channels/xmpp/config.rs`
- `src/channels/xmpp/omemo/`

## Testing Plan

### Unit tests

- channel schema parses `required_fields`
- extension manager returns channel setup fields
- extension configure persists channel fields
- XMPP WASM metadata round-trip

### Integration tests

- install `xmpp` from registry
- configure secrets and fields
- activate against a fake bridge server
- receive DM and reply on same channel
- receive groupchat and reply to room target
- pairing flow for unknown sender

### Bridge tests

- reconnect behavior
- room join behavior
- stanza normalization
- send API behavior
- OMEMO interoperability tests once rewritten

## Recommended First Cut

If the goal is to get to parity fastest, build the first installable XMPP channel with:

- plaintext only
- DM + MUC support
- pairing
- explicit room allowlist
- local bridge over HTTP polling

Then add managed sidecar lifecycle and OMEMO after the installable channel model is working.

That gets XMPP into the same operational shape as `weechat` and `darkirc` without blocking on a full cryptographic rewrite.
