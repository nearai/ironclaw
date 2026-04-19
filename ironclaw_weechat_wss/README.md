# WeeChat Relay Channel for IronClaw

A WeeChat Relay protocol WASM channel for IronClaw agents, providing bridge access to any IRC network via WeeChat.

## Overview

This channel connects IronClaw to WeeChat's API relay (v2), allowing agents to communicate over any IRC network that WeeChat supports (libera, OFTC, ergo, darkirc, etc.).

```
IronClaw ◄─► WeeChat Relay WASM ◄─► WeeChat ◄─► IRC Networks
(Agent)       (HTTP polling)         (relay)      (libera, darkirc, etc.)
```

## Features

- **Multi-network IRC support** - Connect to multiple networks simultaneously via WeeChat
- **HTTP polling mode** - Poll WeeChat API every 3-5 seconds for new messages
- **WebSocket support** (planned) - Real-time push notifications via WebSocket
- **DM and group channels** - Support for both private messages and IRC channels
- **Message chunking** - Automatically splits long responses for IRC line length limits
- **Network filtering** - Allowlist or denylist specific IRC networks
- **Pairing flow** - Secure user approval via `ironclaw pairing approve`
- **Per-buffer watermarks** - Avoids replaying message history on restart

## Quick Start

### 1. WeeChat Setup

In WeeChat, enable the API relay:

```
/relay add api 9001
/set relay.network.password "your-secret-password"
/set relay.network.bind_address "127.0.0.1"
```

Verify relay is running:

```
/relay list
```

### 2. Build WASM Channel

```bash
cd ironclaw_port/weechat_relay
./build.sh
```

Or manually:

```bash
cargo build --target wasm32-wasip2 --release
cp target/wasm32-wasip2/release/weechat_relay_channel.wasm ~/.ironclaw/channels/weechat.wasm
cp weechat.capabilities.json ~/.ironclaw/channels/
```

### 3. Configure IronClaw

The channel will prompt for configuration on first run, or you can edit `~/.ironclaw/channels/weechat.capabilities.json`:

```json
{
  "relay_url": "http://127.0.0.1:9001",
  "relay_password": "your-secret-password",
  "connection_mode": "http",
  "dm_policy": "pairing",
  "group_policy": "allowlist",
  "allow_from": ["your_irc_nick"],
  "networks": [],
  "poll_interval_seconds": 3
}
```

### 4. Restart IronClaw

```bash
ironclaw restart
```

Check logs for:

```
Connected to WeeChat 4.x (API v2)
Found N IRC buffers
```

## Configuration

### Connection Modes

- **`http`** (default) - HTTP polling mode, polls every 3-5 seconds
- **`websocket`** (future) - Real-time WebSocket push notifications

### DM Policy

Controls who can send direct messages to the agent:

- **`open`** - Anyone can message
- **`allowlist`** - Only users in `allow_from` array
- **`pairing`** - Unknown users get a pairing code, approve via CLI

### Group Policy

Controls group/channel message handling:

- **`open`** - Accept from anyone
- **`allowlist`** - Only from users in `allow_from`
- **`deny`** - Ignore all group messages

### Network Filtering

```json
{
  "networks": ["libera", "oftc"],
  "exclude_networks": ["testnet"]
}
```

- `networks` - Allowlist (empty = all networks)
- `exclude_networks` - Denylist

### Sender Allowlist

```json
{
  "allow_from": [
    "sun",
    "alice!~user@example.com",
    "*"
  ]
}
```

Supports:
- Nick only: `"sun"`
- Full hostmask: `"sun!~u@host.example.com"`
- Wildcard: `"*"` (allow all)

## Pairing Flow

When an unknown user messages the agent (with `dm_policy: "pairing"`):

1. User sends DM to bot on IRC
2. Bot replies: `To pair with this agent, run: ironclaw pairing approve weechat CODE123`
3. On IronClaw host, approve: `ironclaw pairing approve weechat CODE123`
4. User can now message the agent

List pending pairing requests:

```bash
ironclaw pairing list weechat
```

## Architecture

### HTTP Polling Mode

Every 3-5 seconds (configurable):

1. WASM channel calls `on_poll()`
2. For each IRC buffer in WeeChat:
   - GET `/api/buffers/<name>/lines?limit=10`
   - Filter for new `irc_privmsg` lines (using watermarks)
   - Parse nick, message text, tags
   - Apply network/sender filters
   - Emit to IronClaw agent via `emit_message()`

3. Agent processes message and generates response
4. WASM channel receives `on_respond()` callback
5. Split response into IRC-sized chunks (420 chars)
6. POST `/api/input` for each chunk to send via WeeChat

### State Management

All state persisted in IronClaw workspace (SQLite):

| Path | Purpose |
|------|---------|
| `state/relay_url` | WeeChat relay endpoint |
| `state/relay_password` | Relay password |
| `state/connection_mode` | `http` or `websocket` |
| `state/last_seen_dates` | Per-buffer watermarks (JSON) |
| `state/buffer_list` | Cached list of IRC buffers (JSON) |
| `state/networks` | Network allowlist (JSON) |
| `state/allow_from` | Sender allowlist (JSON) |

Fresh WASM instance on each callback - no in-memory state.

### Buffer Naming

WeeChat buffer names follow: `irc.<network>.<target>`

Examples:
- `irc.libera.#openclaw` - #openclaw on libera
- `irc.darkirc.sun` - DM with sun on darkirc
- `irc.oftc.#debian` - #debian on OFTC

Server buffers (`irc.server.*`) are filtered out.

## WebSocket Support (Future)

Stub functions are included for future WebSocket implementation:

- `handle_websocket_poll()` - WebSocket health check and message processing
- `_websocket_connect()` - Establish WebSocket connection
- `_websocket_subscribe()` - Subscribe to buffer events
- `_websocket_process_messages()` - Parse WebSocket frames

**Note:** IronClaw's WASM sandbox currently only supports HTTP requests. WebSocket support requires either:

1. Extension to WIT interface for streaming connections
2. WebSocket→HTTP adapter (similar to `darkirc_adapter.py`)

For now, use HTTP polling mode.

## Message Chunking

Long agent responses are split at word/newline boundaries to respect IRC line length limits:

- Default max: 420 characters per chunk
- Configurable via `max_chunk_length`
- UTF-8 safe (checks char boundaries)
- Breaks at newlines first, then spaces
- Hard-splits if no break points available

## Development

### Project Structure

```
weechat_relay/
├── Cargo.toml                    # Rust/WASM dependencies
├── src/
│   └── lib.rs                    # Main channel implementation
├── weechat.capabilities.json     # Channel config schema
├── build.sh                      # Build and install script
└── README.md                     # This file
```

### Building

Requires:
- Rust toolchain with `wasm32-wasip2` target
- IronClaw repo with `wit/channel.wit` interface
- WeeChat >= 4.0 with API relay enabled

```bash
rustup target add wasm32-wasip2
cargo build --target wasm32-wasip2 --release
```

### Testing

Unit tests:

```bash
cargo test
```

Integration test with WeeChat:

```bash
# In WeeChat
/relay add api 9001
/set relay.network.password test123

# Test relay endpoint
curl -u "plain:test123" http://127.0.0.1:9001/api/version
```

### Debugging

Enable IronClaw debug logs:

```bash
RUST_LOG=debug ironclaw
```

Channel debug logs will appear as:

```
[DEBUG weechat] Emitted message from 'sun' in irc.libera.#test (42 chars)
[DEBUG weechat] Sent 2 of 2 chunk(s) to 'irc.libera.#test' (850 chars total)
```

## Troubleshooting

### "Cannot find ironclaw's wit/channel.wit"

The `wit_bindgen` macro expects `../../wit/channel.wit` relative to `Cargo.toml`.

Options:
1. Place this project at `~/ironclaw/channels-src/weechat_relay/`
2. Set `IRONCLAW_REPO=/path/to/ironclaw` and re-run build
3. Symlink: `ln -s /path/to/ironclaw/wit ../wit`

### "HTTP request not allowed: Denied(InsecureScheme)"

IronClaw's WASM sandbox blocks plain HTTP by default. You may need to patch `src/tools/wasm/host.rs` to allow localhost HTTP:

```rust
let validator = if capability.allowlist.iter().any(|p| {
    p.host == "127.0.0.1" || p.host == "localhost" || p.host == "::1"
}) {
    validator.allow_http()
} else {
    validator
};
```

Then rebuild IronClaw binary: `cargo build --release`

### "Invalid capabilities JSON"

Ensure `~/.ironclaw/channels/weechat.capabilities.json` matches the schema. Common issues:

- `"http"` must be a boolean `true`, not an object
- `"secrets"` must be an array of secret names
- `"poll_interval_seconds"` must be >= 3

### No messages received

Check:
1. WeeChat relay is running: `/relay list` in WeeChat
2. Relay password matches config
3. IronClaw logs show polling activity: `Polling tick - calling on_poll channel=weechat`
4. Network filters not excluding desired networks
5. Sender allowlist includes the IRC nick

Test relay manually:

```bash
curl -u "plain:your-password" http://127.0.0.1:9001/api/buffers
```

### Messages sent but not appearing on IRC

Check WeeChat logs for errors:

```
/buffer set localvar set_no_log 0
```

Verify buffer name format:

```bash
curl -u "plain:password" http://127.0.0.1:9001/api/buffers | jq '.[] | .full_name'
```

## Container/Podman Considerations

If IronClaw runs in a container and WeeChat is on the host:

1. **Host gateway** (Podman >= 4.7):
   ```json
   {"relay_url": "http://host.containers.internal:9001"}
   ```

2. **Slirp4netns with loopback**:
   ```ini
   [Container]
   Network=slirp4netns:allow_host_loopback=true
   ```

3. **Tailscale/VPN IP**:
   ```json
   {"relay_url": "http://100.x.y.z:9001"}
   ```

## Security

- Relay password injected by IronClaw host (not stored in WASM)
- HTTP requests restricted to configured relay endpoint (localhost only by default)
- All IronClaw security layers apply (prompt injection defense, rate limiting)
- Pairing flow prevents unauthorized access
- No shell command execution - pure IRC protocol via WeeChat

## License

MIT

## See Also

- [IronClaw Documentation](https://github.com/your-repo/ironclaw)
- [WeeChat Relay Protocol](https://weechat.org/files/doc/stable/weechat_relay_protocol.en.html)
- [WeeChat API Relay](https://weechat.org/doc/api/relay/)
