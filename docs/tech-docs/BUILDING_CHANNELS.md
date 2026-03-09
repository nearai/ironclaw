# Building WASM Channels

> Version baseline: IronClaw v0.16.1 (`v0.16.1` tag snapshot)

This guide covers how to build WASM channel modules for IronClaw.

## Overview

Channels are WASM components that handle communication with external messaging platforms (Telegram, WhatsApp, Slack, etc.). They run in a sandboxed environment and communicate with the host via the WIT (WebAssembly Interface Types) interface.

## Directory Structure

```
channels/                    # Or channels-src/
└── my-channel/
    ├── Cargo.toml
    ├── src/
    │   └── lib.rs
    └── my-channel.capabilities.json
```

After building, deploy to:
```
~/.ironclaw/channels/
├── my-channel.wasm
└── my-channel.capabilities.json
```

## Cargo.toml Template

```toml
[package]
name = "my-channel"
version = "0.1.0"
edition = "2021"
description = "My messaging platform channel for IronClaw"

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.36"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "s"
lto = true
strip = true
codegen-units = 1
```

## Channel Implementation

### Required Imports

```rust
// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",  // Adjust path as needed
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};
```

### Implementing the Guest Trait

```rust
struct MyChannel;

impl Guest for MyChannel {
    /// Called once when the channel starts.
    /// Returns configuration for webhooks and polling.
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        // Parse config from capabilities file
        let config: MyConfig = serde_json::from_str(&config_json)
            .unwrap_or_default();

        Ok(ChannelConfig {
            display_name: "My Channel".to_string(),
            http_endpoints: vec![
                HttpEndpointConfig {
                    path: "/webhook/my-channel".to_string(),
                    methods: vec!["POST".to_string()],
                    require_secret: true,  // Validate webhook secret
                },
            ],
            poll: None,  // Or Some(PollConfig { interval_ms, enabled })
        })
    }

    /// Handle incoming HTTP requests (webhooks).
    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Parse webhook payload
        // Emit messages to agent
        // Return response to webhook caller
    }

    /// Called periodically if polling is enabled.
    fn on_poll() {
        // Fetch new messages from API
        // Emit any new messages
    }

    /// Send a response back to the messaging platform.
    fn on_respond(response: AgentResponse) -> Result<(), String> {
        // Parse metadata to get routing info
        // Call platform API to send message
    }

    /// Called when channel is shutting down.
    fn on_shutdown() {
        channel_host::log(channel_host::LogLevel::Info, "Channel shutting down");
    }

    /// Called when the agent changes state (thinking, processing tools, done, etc.)
    /// Use to send typing indicators or status messages back to the platform.
    fn on_status(update: StatusUpdate) {
        // Called when the agent changes state (thinking, processing tools, done, etc.)
        // Use to send typing indicators or status messages back to the platform.
        // update.status is one of: Thinking, Done, Interrupted, ToolStarted, ToolCompleted,
        // ToolResult, ApprovalNeeded, Status, JobStarted, AuthRequired, AuthCompleted
        let _ = update; // Implement as needed for your platform
    }
}

// Export the channel implementation
export!(MyChannel);
```

## Critical Pattern: Metadata Flow

**The most important pattern**: Store routing info in message metadata so responses can be delivered.

```rust
// When receiving a message, store routing info:
#[derive(Debug, Serialize, Deserialize)]
struct MyMessageMetadata {
    chat_id: String,           // Where to send response
    sender_id: String,         // Who sent it (becomes recipient)
    original_message_id: String,
}

// In on_http_request or on_poll:
let metadata = MyMessageMetadata {
    chat_id: message.chat.id.clone(),
    sender_id: message.from.clone(),  // CRITICAL: Store sender!
    original_message_id: message.id.clone(),
};

channel_host::emit_message(&EmittedMessage {
    user_id: message.from.clone(),
    user_name: Some(name),
    content: text,
    thread_id: None,
    metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
});

// In on_respond, use the ORIGINAL message's metadata:
fn on_respond(response: AgentResponse) -> Result<(), String> {
    let metadata: MyMessageMetadata = serde_json::from_str(&response.metadata_json)?;

    // sender_id becomes the recipient!
    send_message(metadata.chat_id, metadata.sender_id, response.content);
}
```

## Credential Injection

**Never hardcode credentials!** Use placeholders that the host replaces:

### URL Placeholders (Telegram-style)

```rust
// The host replaces {TELEGRAM_BOT_TOKEN} with the actual token
let url = "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage";
channel_host::http_request("POST", url, &headers_json, Some(body.into_bytes()), None)?;
```

### Header Placeholders (WhatsApp-style)

```rust
// The host replaces {WHATSAPP_ACCESS_TOKEN} in headers too
let headers = serde_json::json!({
    "Content-Type": "application/json",
    "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
});
channel_host::http_request("POST", &url, &headers.to_string(), Some(body.into_bytes()), None)?;
```

The placeholder format is `{SECRET_NAME}` where `SECRET_NAME` matches the credential name in uppercase with underscores (e.g., `whatsapp_access_token` → `{WHATSAPP_ACCESS_TOKEN}`).

### Host-Based Credential Injection (v0.13.0)

As of v0.13.0, credentials can also be injected automatically at the host boundary without requiring placeholder syntax in WASM code. Declare credentials in the capabilities file under `capabilities.http.credentials`:

```json
"credentials": {
  "my_service_token": {
    "secret_name": "my_api_key",
    "location": { "type": "bearer" },
    "host_patterns": ["api.myservice.com"]
  }
}
```

When the WASM channel makes an HTTP request to a matching host, the ironclaw runtime automatically injects the credential as a Bearer token header. The WASM code never sees the raw secret value.

**Credential injection location types:**

| Type | Description | Example |
|------|-------------|---------|
| `bearer` | `Authorization: Bearer {token}` | `{"type": "bearer"}` |
| `header` | Custom header with optional prefix | `{"type": "header", "name": "X-API-Key"}` or `{"type": "header", "name": "Authorization", "prefix": "Token "}` |
| `basic` | HTTP Basic auth (token as password) | `{"type": "basic", "username": "myapp"}` |
| `query_param` | URL query parameter | `{"type": "query_param", "name": "api_key"}` |
| `url_path` | URL path placeholder replacement | `{"type": "url_path", "placeholder": "USER_ID"}` |

### OAuth Credential Injection (v0.15.0)

WASM tools can declare OAuth 2.0 flows in the capabilities file under the `auth.oauth` section:

```json
"auth": {
  "secret_name": "my_service_token",
  "display_name": "My Service",
  "oauth": {
    "authorization_url": "https://api.myservice.com/oauth/authorize",
    "token_url": "https://api.myservice.com/oauth/token",
    "scopes": ["read:data", "write:data"],
    "use_pkce": true,
    "extra_params": {
      "access_type": "offline"
    },
    "client_id_env": "MY_SERVICE_CLIENT_ID",
    "client_secret_env": "MY_SERVICE_CLIENT_SECRET",
    "access_token_field": "access_token",
    "validation_endpoint": {
      "url": "https://api.myservice.com/v1/me",
      "method": "GET",
      "success_status": 200,
      "headers": {}
    }
  }
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `authorization_url` | string | required | OAuth 2.0 authorization endpoint |
| `token_url` | string | required | Token exchange endpoint |
| `scopes` | string[] | `[]` | OAuth scopes to request |
| `use_pkce` | bool | `true` | Enable PKCE (recommended for CLI) |
| `extra_params` | object | `{}` | Additional auth URL parameters (e.g., `access_type`, `approval_prompt`) |
| `client_id_env` | string | — | Env var for client ID override |
| `client_secret_env` | string | — | Env var for client secret override |
| `access_token_field` | string | `"access_token"` | Field name in token response |
| `validation_endpoint.url` | string | — | URL to verify token is valid for correct account |
| `validation_endpoint.method` | string | `"GET"` | HTTP method for validation |
| `validation_endpoint.success_status` | number | `200` | Expected HTTP status for valid token |
| `validation_endpoint.headers` | object | `{}` | Custom headers (e.g., `{"Notion-Version": "2022-06-28"}`) |

**Google OAuth built-in defaults:** Tools using `secret_name: "google_oauth_token"` get built-in Google OAuth credentials automatically — no need to register your own OAuth app. Override at runtime with `GOOGLE_OAUTH_CLIENT_ID` / `GOOGLE_OAUTH_CLIENT_SECRET` env vars.

**OAuth callback server:** The callback listens on `127.0.0.1:9876` by default. For remote server deployments, set `IRONCLAW_OAUTH_CALLBACK_URL` to your server's accessible callback URL and optionally `OAUTH_CALLBACK_HOST` to change the bind interface.

When configured, the ironclaw runtime handles the full OAuth 2.0 flow (authorization, token exchange, scope merging for shared providers) without the WASM code implementing it. The resulting access token is stored in the secrets store and injected via the declared credential injection mechanism.

Tools sharing the same OAuth provider (e.g., two Google-based tools) have their scopes merged automatically, triggering a single re-authorization with consolidated permissions.

## Capabilities File

Create `my-channel.capabilities.json`:

```json
{
  "version": "0.1.0",
  "wit_version": "0.2.0",
  "type": "channel",
  "name": "my-channel",
  "description": "My messaging platform channel",
  "setup": {
    "required_secrets": [
      {
        "name": "my_channel_api_token",
        "prompt": "Enter your API token",
        "optional": false
      },
      {
        "name": "my_channel_webhook_secret",
        "prompt": "Webhook secret",
        "optional": true
      }
    ]
  },
  "capabilities": {
    "http": {
      "allowlist": [
        { "host": "api.my-platform.com", "path_prefix": "/" }
      ],
      "rate_limit": {
        "requests_per_minute": 60,
        "requests_per_hour": 1000
      }
    },
    "secrets": {
      "allowed_names": ["my_channel_*"]
    },
    "channel": {
      "allowed_paths": ["/webhook/my-channel"],
      "allow_polling": false,
      "workspace_prefix": "channels/my-channel/",
      "emit_rate_limit": {
        "messages_per_minute": 100,
        "messages_per_hour": 5000
      },
      "webhook": {
        "secret_header": "X-Webhook-Secret",
        "secret_name": "my_channel_webhook_secret"
      }
    }
  },
  "config": {
    "custom_option": "value"
  }
}
```

## Building and Deploying

### Supply Chain Security: No Committed Binaries

**Do not commit compiled WASM binaries.** They are a supply chain risk — the binary in a PR may not match the source. IronClaw builds channels from source:

- `cargo build` automatically builds `telegram.wasm` via `build.rs`
- The built binary is in `.gitignore` and is not committed
- CI should run `cargo build` (or `./scripts/build-all.sh`) to produce releases

**Reproducible build:**
```bash
cargo build --release
```

Prerequisites: `rustup target add wasm32-wasip2`, `cargo install wasm-tools` (optional; fallback copies raw WASM if unavailable).

### Telegram Channel (Manual Build)

```bash
# Add WASM target if needed
rustup target add wasm32-wasip2

# Build Telegram channel
./channels-src/telegram/build.sh

# Install (or use ironclaw onboard to install bundled channel)
mkdir -p ~/.ironclaw/channels
cp channels-src/telegram/telegram.wasm channels-src/telegram/telegram.capabilities.json ~/.ironclaw/channels/
```

**Note**: The main IronClaw build compiles Telegram channel artifacts via `build.rs`, but does not embed `telegram.wasm` with `include_bytes!`. Manual `./channels-src/telegram/build.sh` is optional (useful for direct channel-only iteration).

### Other Channels

```bash
# Build the WASM component
cd channels-src/my-channel
cargo build --release --target wasm32-wasip2

# Deploy to ~/.ironclaw/channels/
cp target/wasm32-wasip2/release/my_channel.wasm ~/.ironclaw/channels/my-channel.wasm
cp my-channel.capabilities.json ~/.ironclaw/channels/
```

## Host Functions Available

The channel host provides these functions:

```rust
// Logging
channel_host::log(LogLevel::Info, "Message");

// Time
let now = channel_host::now_millis();

// Workspace (scoped to channel namespace)
let data = channel_host::workspace_read("state/offset");
channel_host::workspace_write("state/offset", "12345")?;

// HTTP requests (credentials auto-injected)
let response = channel_host::http_request("POST", &url, &headers, Some(body.into_bytes()), None)?;

// Emit message to agent
channel_host::emit_message(&EmittedMessage { ... });

// Check if a secret exists (without reading its value)
let exists = channel_host::secret_exists("my_api_key");
```

> **v0.14.0 (#479):** Secrets are now available throughout the WASM tool lifecycle, including during `on_start()` initialization. You can safely call `channel_host::secret_exists(name)` during startup to check credential availability before registering tools.

```rust
// DM Pairing — check and manage which users are allowed to interact
let result = channel_host::pairing_upsert_request(channel, id, &meta_json)?;
// result.code: String (display to user for confirmation), result.created: bool
let allowed = channel_host::pairing_is_allowed(channel, id, Some(username))?;
let allow_list = channel_host::pairing_read_allow_from(channel)?;
```

Used to implement allow-list-based access control. Call `pairing_is_allowed` at message receipt to gate access. Call `pairing_upsert_request` to create a pairing code for users to confirm.

## Common Patterns

### Webhook Secret Validation

The host validates webhook secrets automatically. Check `req.secret_validated`:

```rust
fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
    if !req.secret_validated {
        channel_host::log(LogLevel::Warn, "Invalid webhook secret");
        // Host should have already rejected, but defense in depth
    }
    // ...
}
```

### Polling with Offset Tracking

For platforms that require polling (not webhook-based):

```rust
const OFFSET_PATH: &str = "state/last_offset";

fn on_poll() {
    // Read last offset
    let offset = channel_host::workspace_read(OFFSET_PATH)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    // Fetch updates since offset
    let updates = fetch_updates(offset);

    // Process and track new offset
    let mut new_offset = offset;
    for update in updates {
        if update.id >= new_offset {
            new_offset = update.id + 1;
        }
        emit_message(update);
    }

    // Save new offset
    if new_offset != offset {
        let _ = channel_host::workspace_write(OFFSET_PATH, &new_offset.to_string());
    }
}
```

### Status Message Filtering

Skip status updates to prevent loops:

```rust
// Skip status updates (delivered, read, etc.)
if !payload.statuses.is_empty() && payload.messages.is_empty() {
    return;  // Only status updates, no actual messages
}
```

### Bot Message Filtering

Skip bot messages to prevent infinite loops:

```rust
if sender.is_bot {
    return;  // Don't respond to bots
}
```

## Testing

Add tests in the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook() {
        let json = r#"{ ... }"#;
        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.messages.len(), 1);
    }

    #[test]
    fn test_metadata_roundtrip() {
        let meta = MyMessageMetadata { ... };
        let json = serde_json::to_string(&meta).unwrap();
        let parsed: MyMessageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta.chat_id, parsed.chat_id);
    }
}
```

Run tests with:
```bash
cargo test
```

## Troubleshooting

### "byte index N is not a char boundary"

Never slice strings by byte index! Use character-aware truncation:

```rust
// BAD: panics on multi-byte UTF-8 (emoji, etc.)
let preview = &content[..50];

// GOOD: safe truncation
let preview: String = content.chars().take(50).collect();
```

### Credential placeholders not replaced

1. Check the secret name matches (lowercase with underscores)
2. Verify the secret is in `allowed_names` in capabilities
3. Check logs for "unresolved placeholders" warnings

### Messages not routing to responses

Ensure `on_respond` uses the ORIGINAL message's metadata, not response metadata:
```rust
// response.metadata_json comes from the ORIGINAL emit_message call
let metadata: MyMetadata = serde_json::from_str(&response.metadata_json)?;
```

---

## WIT Interface Versioning (v0.16.0)

IronClaw v0.16.0 introduced WIT interface versioning. Your `capabilities.json` must declare the WIT version it was compiled against:

```json
{
  "version": "0.1.0",
  "wit_version": "0.2.0",
  ...
}
```

**Current WIT versions:**

| Interface | Version |
|-----------|---------|
| `wit/tool.wit` | `0.2.0` |
| `wit/channel.wit` | `0.2.0` |

If your `wit_version` does not match the host's version, the `extension_info` tool will report a mismatch:

```json
{
  "wit_version": "0.1.0",
  "host_wit_version": "0.2.0"
}
```

When this happens, recompile your WASM module against the current WIT files:

```bash
# Pull the latest WIT files
cd ~/src/ironclaw
git pull

# Rebuild
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/my_channel.wasm ~/.ironclaw/channels/my-channel.wasm
```

Then update `"wit_version"` in your `capabilities.json` to match.

### HMAC-SHA256 Webhook Security (Slack-style channels, v0.16.0)

For channels receiving webhooks from Slack-compatible services, add `hmac_secret_name` to the `webhook` block in your capabilities:

```json
"channel": {
  "allowed_paths": ["/webhook/my-channel"],
  "webhook": {
    "hmac_secret_name": "my_channel_signing_secret"
  }
}
```

The host will:
1. Look up the secret named `my_channel_signing_secret` at activation time.
2. On every incoming webhook, verify the `X-Slack-Signature` header using HMAC-SHA256.
3. Reject requests with timestamps older than 5 minutes (replay protection).
4. Only call `on_http_request` if the signature is valid.
