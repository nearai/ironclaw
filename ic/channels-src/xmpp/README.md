# XMPP WASM Channel

This directory contains the installable `xmpp` WASM channel package.

It is the IronClaw-facing adapter layer for XMPP:

- `xmpp.wasm` is installed into `~/.ironclaw/channels/`
- `xmpp.capabilities.json` defines setup secrets, setup fields, and HTTP permissions
- the channel talks to a local `xmpp-bridge` process over loopback HTTP

The actual XMPP protocol session, MUC membership, reconnect loop, and current
OMEMO behavior live in [bridges/xmpp-bridge](/home/cmc/ironclaw/bridges/xmpp-bridge).
Configured `rooms` are auto-joined by the bridge on connect.
Configured `encrypted_rooms` are treated as fail-closed encrypted groupchats:
they are validated as non-anonymous, members-only rooms, member/admin/owner
real JIDs are cached from MUC state, and plaintext groupchat traffic is ignored.

## Build

```bash
./channels-src/xmpp/build.sh
```

That produces `channels-src/xmpp/xmpp.wasm`.

## Install

```bash
mkdir -p ~/.ironclaw/channels
cp channels-src/xmpp/xmpp.wasm channels-src/xmpp/xmpp.capabilities.json ~/.ironclaw/channels/
```

## Configure

The channel expects these secrets:

- `xmpp_password`
- `xmpp_bridge_token` (optional)

The channel expects these setup fields:

- `xmpp_jid`
- `bridge_url`
- `dm_policy`
- `allow_from`
- `rooms`
- `encrypted_rooms`
- `allow_plaintext_fallback`
- `max_messages_per_hour`
- `resource`
- `device_id`
- `omemo_store_dir`

Use `ironclaw onboard`, the channel setup UI, or extension configuration flows
to persist them. `max_messages_per_hour` caps outbound XMPP sends per bot
instance; set it to `0` to disable the cap.
