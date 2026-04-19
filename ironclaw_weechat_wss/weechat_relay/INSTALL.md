# Installation Guide

## Prerequisites

- [IronClaw](https://github.com/ironclaw) installed and running
- [WeeChat](https://weechat.org) >= 4.0 with API relay enabled
- Rust toolchain with `wasm32-wasip2` target
- Python 3 with `aiohttp` (`pip install aiohttp`)

---

## 1. Enable WeeChat Relay

In WeeChat:

```
/relay add api 9001
/set relay.network.password "your-secret-password"
/set relay.network.bind_address "127.0.0.1"
/relay list
```

---

## 2. Build and Install the WASM Channel

```bash
cd weechat_relay
./build.sh
```

This compiles the WASM and copies it plus `weechat.capabilities.json` to `~/.ironclaw/channels/`.

---

## 3. Configure

Edit `~/.ironclaw/channels/weechat.capabilities.json` and set your relay password and allowed users:

```json
{
  "relay_url": "http://127.0.0.1:9001",
  "relay_password": "your-secret-password",
  "connection_mode": "auto",
  "ws_adapter_url": "http://127.0.0.1:6681",
  "dm_policy": "pairing",
  "group_policy": "allowlist",
  "allow_from": ["yournick", "othernick!~user@host.example"]
}
```

**`allow_from`** accepts bare nicks or full `nick!user@host` hostmasks. Use `["*"]` to allow anyone.

**`dm_policy`** options:
- `"pairing"` — unknown users get a pairing code, approve with `ironclaw pairing approve weechat CODE`
- `"open"` — anyone can DM the agent

**`group_policy`** options:
- `"allowlist"` — only nicks in `allow_from` trigger the agent in channels
- `"open"` — everyone in every channel triggers the agent
- `"deny"` — ignore all channel messages

---

## 4. Start the WebSocket Adapter

The adapter maintains a persistent WebSocket connection to WeeChat for real-time message delivery:

```bash
cd weechat_relay
RELAY_PASSWORD=your-secret-password python3 ws_adapter.py
```

Keep this running in a terminal or as a service. It listens on `http://127.0.0.1:6681` by default.

---

## 5. Restart IronClaw

```bash
ironclaw restart
```

Verify the channel loaded:

```bash
ironclaw channels list
```

---

## 6. Pairing (first-time users)

If `dm_policy` is `"pairing"`, when a new user DMs the bot they receive a code in IRC. Approve it on the host:

```bash
ironclaw pairing approve weechat CODE123
ironclaw pairing list weechat    # see pending requests
```

---

## Troubleshooting

**Test relay connectivity:**
```bash
curl -u "plain:your-password" http://127.0.0.1:9001/api/version
```

**No messages received:**
- Check the adapter is running and connected: `curl http://127.0.0.1:6681/api/health`
- Check `allow_from` includes the sender's nick
- Run `ironclaw` with `RUST_LOG=debug` for verbose logs

**"HTTP not allowed" error:**
IronClaw's WASM sandbox may need to be patched to allow localhost HTTP.
