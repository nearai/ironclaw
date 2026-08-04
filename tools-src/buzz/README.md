# Buzz Messaging Tool for IronClaw

WASM tool for Buzz messaging via Nostr. Send messages to channels, subscribe to events, reply to threads, and mention users.

## Features

- **Send Messages** - Post to Buzz channels with thread replies and mentions
- **Subscribe** - Listen for new events in a channel via Nostr relay
- **Threading** - Reply to specific events via NIP-01 `#e` tags
- **Mentions** - `@mention` users via NIP-01 `#p` tags

All signing is handled by the IronClaw host — your Nostr private key never enters the WASM sandbox.

## Setup

1. Store your Nostr private key:

   ```
   ironclaw secret set buzz_private_key nsec1...
   ```

   Hex format also works (64 hex chars).

2. The tool requires a `nostr` capability in its capabilities file:

   ```json
   {
     "nostr": {
       "secret_name": "buzz_private_key"
     }
   }
   ```

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

## Building

```bash
cd tools-src/buzz
cargo build --target wasm32-unknown-unknown --release
```

The output is `target/wasm32-unknown-unknown/release/buzz_tool.wasm`.

## Architecture

Buzz is a pure WASM tool — all Nostr operations go through the IronClaw host:

```
WASM (Buzz)                    Host (IronClaw)
    │                              │
    ├── nostr_sign_event() ──────► │ holds private key, signs
    ├── nostr_publish_event() ───► │ opens WS, sends EVENT, reads OK
    └── nostr_subscribe_events() ► │ opens WS, sends REQ, collects events
```

The tool only constructs unsigned events and relay URLs. The host handles all crypto and network I/O.

## License

MIT/Apache-2.0
