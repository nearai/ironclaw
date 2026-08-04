# Buzz Messaging Tool for IronClaw

WASM tool for Buzz messaging via Nostr. Send messages to channels, subscribe to events, reply to threads, and mention users.

## Features

- **Send Messages** - Post to Buzz channels with thread replies and mentions
- **Subscribe** - Listen for new events in a channel via Nostr relay
- **Threading** - Reply to specific events via NIP-01 `#e` tags
- **Mentions** - `@mention` users via NIP-01 `#p` tags

All signing is handled by the IronClaw host — your Nostr private key never enters the WASM sandbox.

## Prerequisites

- [Rust](https://rustup.rs/) with the `wasm32-wasip2` target:
  ```bash
  rustup target add wasm32-wasip2
  ```
- An IronClaw binary (built from this repo)
- A Nostr keypair (nsec / npub)

## Setup

### 1. Build the WASM tool

From the repo root:

```bash
cd tools-src/buzz
cargo build --target wasm32-wasip2 --release
```

Output: `target/wasm32-wasip2/release/buzz_tool.wasm`

### 2. Create the capabilities file

IronClaw tools declare their required permissions via a capabilities JSON sidecar.

Create `tools-src/buzz/buzz-tool.capabilities.json`:

```json
{
  "nostr": {
    "secret_name": "buzz_private_key"
  }
}
```

The `secret_name` must match the secret you store in step 3.

### 3. Store your Nostr private key

**Option A: IronClaw secrets store** (persistent)

```
ironclaw secret set buzz_private_key nsec1...
```

Hex format also works (64 hex characters).

**Option B: Environment variable** (for ACP serve)

```bash
export BUZZ_PRIVATE_KEY=nsec1...
```

ACP serve auto-seeds this into the secrets store on startup.

### 4. Run IronClaw

**Production (loads from `~/.ironclaw/tools/`):**

```bash
# Copy the built artifacts to the tools directory
mkdir -p ~/.ironclaw/tools
cp tools-src/buzz/target/wasm32-wasip2/release/buzz_tool.wasm ~/.ironclaw/tools/
cp tools-src/buzz/buzz-tool.capabilities.json ~/.ironclaw/tools/buzz_tool.capabilities.json

# Start IronClaw
ironclaw serve
```

The tool will appear as `buzz_tool` in the agent's available tools.

**Dev mode (loads from build output automatically):**

```bash
# From the repo root — just start IronClaw
# It auto-discovers tools-src/*/target/wasm32-wasip2/release/*.wasm
ironclaw serve
```

Dev builds take priority over installed copies. No need to copy files.

**ACP serve (for local AI agents):**

```bash
export BUZZ_PRIVATE_KEY=nsec1...
ironclaw acp serve
```

Buzz is loaded automatically from the build output. `--dev-tools` is optional — it enables shell and file editing tools for the agent, unrelated to Buzz.

## Usage Examples

### Send a Message

```json
{
  "action": "send_message",
  "channel_id": "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f",
  "content": "Hello from IronClaw!"
}
```

### Reply to a Thread

```json
{
  "action": "send_message",
  "channel_id": "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f",
  "content": "Replying to the thread",
  "reply_to_event_id": "abc123..."
}
```

### Mention Users

```json
{
  "action": "send_message",
  "channel_id": "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f",
  "content": "Hey @alice, check this out",
  "mention_pubkeys": ["npub1..."]
}
```

### Subscribe to a Channel

```json
{
  "action": "subscribe_channel",
  "channel_id": "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f",
  "timeout_ms": 5000,
  "limit": 20
}
```

### Custom Relay

Both actions accept an optional `relay_url` field. Defaults to `wss://nearbuilders.communities.buzz.xyz`.

```json
{
  "action": "send_message",
  "channel_id": "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f",
  "content": "hello",
  "relay_url": "wss://my-relay.example.com"
}
```

## Architecture

Buzz is a pure WASM tool — all Nostr operations go through the IronClaw host:

```
WASM (Buzz)                    Host (IronClaw)
    │                              │
    ├── nostr_sign_event() ──────► │ holds private key, signs with schnorr
    ├── nostr_publish_event() ───► │ opens WS, sends EVENT, reads OK
    └── nostr_subscribe_events() ► │ opens WS, sends REQ, collects events
```

The tool only constructs unsigned events and relay URLs. The host handles all crypto and network I/O.

## File Layout

```
tools-src/buzz/
├── Cargo.toml                          # crate config
├── README.md                          # this file
├── buzz-tool.capabilities.json        # permissions declaration
└── src/
    └── lib.rs                         # Buzz tool implementation
```

Built artifacts:
```
tools-src/buzz/target/wasm32-wasip2/release/buzz_tool.wasm
```

## License

MIT/Apache-2.0
