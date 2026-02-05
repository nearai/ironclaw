# Slack WASM Tool

A standalone WASM component that provides Slack integration for IronClaw. This serves as both a functional tool and a template for building custom WASM tools.

## Features

- **send_message**: Send messages to channels or threads
- **list_channels**: List channels the bot has access to
- **get_channel_history**: Retrieve recent messages from a channel
- **post_reaction**: Add emoji reactions to messages
- **get_user_info**: Get information about Slack users

## Prerequisites

1. **Rust toolchain** with WASM target:
   ```bash
   rustup target add wasm32-wasip2
   ```

2. **cargo-component** for building WASM components:
   ```bash
   cargo install cargo-component
   ```

3. **Slack Bot Token** with the following OAuth scopes:
   - `chat:write` - Send messages
   - `channels:read` - List public channels
   - `channels:history` - Read channel history
   - `groups:read` - List private channels
   - `groups:history` - Read private channel history
   - `reactions:write` - Add reactions
   - `users:read` - Get user information

## Building

```bash
cd examples/wasm-tools/slack
cargo component build --release
```

The compiled WASM component will be at:
```
target/wasm32-wasip2/release/slack_tool.wasm
```

## Installation

### Option A: File-based (Development)

Copy the WASM and capabilities files to the agent's tools directory:

```bash
mkdir -p ~/.ironclaw/tools
cp target/wasm32-wasip2/release/slack_tool.wasm ~/.ironclaw/tools/slack.wasm
cp slack.capabilities.json ~/.ironclaw/tools/
```

### Option B: Database Storage (Production)

Use the agent CLI or API to store the tool:

```bash
ironclaw tool install \
  --name slack \
  --wasm target/wasm32-wasip2/release/slack_tool.wasm \
  --capabilities slack.capabilities.json
```

## Configuration

Store your Slack bot token as a secret:

```bash
ironclaw secret set slack_bot_token "xoxb-your-token-here"
```

Or via SQL:
```sql
INSERT INTO secrets (user_id, name, encrypted_value, key_salt)
VALUES ('your_user_id', 'slack_bot_token', ...);
```

## Usage Examples

### Send a Message

```json
{
  "action": "send_message",
  "channel": "#general",
  "text": "Hello from IronClaw!"
}
```

### Reply in a Thread

```json
{
  "action": "send_message",
  "channel": "C1234567890",
  "text": "This is a thread reply",
  "thread_ts": "1234567890.123456"
}
```

### List Channels

```json
{
  "action": "list_channels",
  "limit": 50
}
```

### Get Channel History

```json
{
  "action": "get_channel_history",
  "channel": "C1234567890",
  "limit": 10
}
```

### Add a Reaction

```json
{
  "action": "post_reaction",
  "channel": "C1234567890",
  "timestamp": "1234567890.123456",
  "emoji": "thumbsup"
}
```

### Get User Info

```json
{
  "action": "get_user_info",
  "user_id": "U1234567890"
}
```

## Security Model

This tool runs in a sandboxed WASM environment with strict capability controls:

1. **HTTP Allowlist**: Can only access `slack.com/api/*`
2. **Credential Injection**: The bot token is injected by the host runtime; the WASM code never sees it
3. **Rate Limiting**: 50 requests/minute, 1000 requests/hour
4. **No Filesystem Access**: Cannot read/write files except through workspace capability
5. **No Network Access**: Beyond the allowlisted endpoints

## Capabilities File

The `slack.capabilities.json` file declares what this tool needs:

```json
{
  "http": {
    "allowlist": [
      { "host": "slack.com", "path_prefix": "/api/", "methods": ["GET", "POST"] }
    ],
    "credentials": {
      "slack_bot_token": {
        "secret_name": "slack_bot_token",
        "location": { "type": "bearer" },
        "host_patterns": ["slack.com"]
      }
    },
    "rate_limit": { "requests_per_minute": 50, "requests_per_hour": 1000 }
  },
  "secrets": {
    "allowed_names": ["slack_bot_token"]
  }
}
```

## Building Your Own Tool

Use this as a template for creating new WASM tools:

1. Copy this directory
2. Update `Cargo.toml` with your tool name
3. Modify `src/types.rs` with your action types
4. Implement API calls in `src/api.rs`
5. Update the action dispatch in `src/lib.rs`
6. Create your `*.capabilities.json` file
7. Build with `cargo component build --release`

### Key Files

- `Cargo.toml` - Rust package config with WASM target
- `src/lib.rs` - WIT bindings and main dispatch
- `src/types.rs` - Request/response types
- `src/api.rs` - API implementation
- `*.capabilities.json` - Security capabilities declaration

### WIT Interface

Tools implement the `sandboxed-tool` world from `wit/tool.wit`:

```wit
world sandboxed-tool {
    import host;   // log, http-request, secret-exists, etc.
    export tool;   // execute, schema, description
}
```

## Troubleshooting

### "Slack bot token not configured"

Ensure you've stored the secret:
```bash
ironclaw secret set slack_bot_token "xoxb-..."
```

### "Endpoint not in allowlist"

Check that `slack.capabilities.json` includes the endpoint you're trying to access.

### "Rate limit exceeded"

The tool has a default rate limit of 50 requests/minute. Wait and retry.

### Build errors

Ensure you have the WASM target and cargo-component installed:
```bash
rustup target add wasm32-wasip2
cargo install cargo-component
```

## License

MIT OR Apache-2.0
